/**
 * The result a WebMCP tool call of a Live action returns (AGENT-012): the
 * outcome, the fields the island root lists as agent-visible, copied from
 * the committed snapshot's state, and any validation issues. Nothing else
 * of the island's state, its HTML, its events or its effects reaches the
 * agent. A 64-bit integer field, which the snapshot carries as a lossless
 * tag, is shown as its decimal string, as the server's headless result
 * shows it.
 */
import type { ValidatedResponse } from "../application/types.js";
import type { ModelContextToolResult } from "./model-context.js";

const INTEGER_TAG = "$live";

function record(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

/** `value`, or the decimal string of a lossless 64-bit integer tag. */
function shown(value: unknown): unknown {
  const tagged = record(value);
  if (
    tagged !== null &&
    Object.keys(tagged).length === 2 &&
    (tagged[INTEGER_TAG] === "u64" || tagged[INTEGER_TAG] === "i64") &&
    typeof tagged["value"] === "string"
  ) {
    return tagged["value"];
  }
  return value;
}

/** The agent-visible fields of `snapshot`'s state, and no other. */
export function agentFields(
  snapshot: Readonly<Record<string, unknown>>,
  fields: readonly string[],
): Record<string, unknown> {
  const state = record(record(snapshot["body"])?.["state"]);
  const out: Record<string, unknown> = {};
  if (state === null) return out;
  for (const field of fields) {
    if (Object.prototype.hasOwnProperty.call(state, field)) out[field] = shown(state[field]);
  }
  return out;
}

function result(structured: Record<string, unknown>, isError: boolean): ModelContextToolResult {
  return Object.freeze({
    content: Object.freeze([
      Object.freeze({ type: "text" as const, text: JSON.stringify(structured) }),
    ]),
    structuredContent: Object.freeze(structured),
    isError,
  });
}

/** A tool error with `message`, for a call that did not reach an outcome. */
export function agentToolError(message: string): ModelContextToolResult {
  return result({ outcome: "error", message }, true);
}

/** The result of a call whose action request the island applied. */
export function agentToolResult(
  response: ValidatedResponse,
  fields: readonly string[],
): ModelContextToolResult {
  switch (response.kind) {
    case "committed": {
      const validation = response.validation;
      const invalid = Object.keys(validation).length > 0;
      const outcome = invalid
        ? "invalid"
        : response.render.kind === "no_render"
          ? "no_render"
          : "accepted";
      const structured: Record<string, unknown> = {
        outcome,
        fields: agentFields(response.snapshot, fields),
      };
      if (invalid) structured["validation"] = validation;
      return result(structured, invalid);
    }
    case "navigation":
      return result({ outcome: "redirect", fields: {} }, false);
    case "rejected":
    case "recovery":
    case "fatal":
      return result(
        {
          outcome: "error",
          message:
            response.kind === "rejected"
              ? "The action did not run: call again."
              : "The action did not run, and the island will refresh.",
        },
        true,
      );
  }
}
