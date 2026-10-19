/**
 * The result a WebMCP tool call of a Live action returns (AGENT-012): the
 * outcome, the fields the island root lists as agent-visible, copied from
 * the committed snapshot's state, and the validation issues the agent may
 * read. Nothing else
 * of the island's state, its HTML, its events or its effects reaches the
 * agent. A 64-bit integer field, which the snapshot carries as a lossless
 * tag and the island root marks, is shown as its decimal string, as the
 * server's headless result shows it.
 */
import type { ValidatedResponse } from "../application/types.js";
import type { AgentToolDefinition } from "../islands/metadata.js";
import type { ModelContextToolResult } from "./model-context.js";

const INTEGER_TAG = "$live";

function record(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

/** `value`, or the decimal string of `value` when it is a lossless 64-bit integer tag. */
function unwrapped(value: unknown): unknown {
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

/**
 * The agent-visible fields of `snapshot`'s state, and no other. Only a
 * field in `integers`, which the server marks as holding a 64-bit integer,
 * is unwrapped from its lossless tag; any other field, a Json field
 * holding the same shape included, is shown as the snapshot carries it.
 */
export function agentFields(
  snapshot: Readonly<Record<string, unknown>>,
  fields: readonly string[],
  integers: readonly string[],
): Record<string, unknown> {
  const state = record(record(snapshot["body"])?.["state"]);
  const out: Record<string, unknown> = {};
  if (state === null) return out;
  for (const field of fields) {
    if (!Object.prototype.hasOwnProperty.call(state, field)) continue;
    out[field] = integers.includes(field) ? unwrapped(state[field]) : state[field];
  }
  return out;
}

/** The words of one validation issue: its text, else its message identity. */
function message(issue: unknown): string {
  if (typeof issue === "string") return issue;
  const entry = record(issue);
  const words = entry?.["text"] ?? entry?.["id"];
  return typeof words === "string" ? words : "";
}

/**
 * Each validation issue the agent may read, by path, with its words: an
 * issue on an agent-visible field or on one of the tool's arguments. An
 * issue on any other path is reported once, under the empty path and with
 * no message, so the agent learns the call failed validation without
 * reading a field it cannot see. The server's headless result reports
 * issues the same way.
 */
function shownValidation(
  validation: Readonly<Record<string, unknown>>,
  scope: AgentResultScope,
): Record<string, string[]> {
  const shown = new Set([...scope.fields, ...scope.arguments]);
  const out: Record<string, string[]> = {};
  let hidden = false;
  for (const [path, issue] of Object.entries(validation)) {
    const root = path.split(".", 1)[0] ?? "";
    if (shown.has(root)) out[path] = [message(issue)];
    else hidden = true;
  }
  if (hidden) out[""] = [];
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

/**
 * What a tool result may show of an island: its agent-visible fields, and
 * the names of the tool's own arguments, which are the agent's own input.
 */
export interface AgentResultScope {
  readonly fields: readonly string[];
  /** The agent-visible fields holding a 64-bit integer. */
  readonly integers: readonly string[];
  readonly arguments: readonly string[];
}

/** The names of `tool`'s arguments: the properties its input schema lists. */
export function argumentNames(tool: AgentToolDefinition): string[] {
  const properties = record(tool.inputSchema["properties"]);
  return properties === null ? [] : Object.keys(properties);
}

/** The result of a call whose action request the island applied. */
export function agentToolResult(
  response: ValidatedResponse,
  scope: AgentResultScope,
): ModelContextToolResult {
  const fields = scope.fields;
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
        fields: agentFields(response.snapshot, fields, scope.integers),
      };
      if (invalid) structured["validation"] = shownValidation(validation, scope);
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
