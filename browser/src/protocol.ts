import { parseCanonicalJson } from "./canonical.js";
import { asArray, asNumber, asRecord, asString, requireExactKeys } from "./schema.js";

export class ProtocolValidationError extends Error {
  public constructor(public readonly code: string) {
    super(code);
    this.name = "ProtocolValidationError";
  }
}

export function validateUpdateRequest(text: string): void {
  let root: Readonly<Record<string, unknown>>;
  try {
    root = asRecord(parseCanonicalJson(text));
  } catch (error: unknown) {
    const code = error instanceof Error ? error.message : "invalid_protocol_envelope";
    throw new ProtocolValidationError(code === "duplicate_key" ? "protocol_duplicate_field" : code);
  }
  requireExactKeys(root, [
    "base_revision",
    "component",
    "correlation_id",
    "extensions",
    "idempotency_key",
    "model_proposals",
    "operations",
    "protocol_version",
    "runtime_contract_version",
    "snapshot",
    "snapshot_schema_version",
  ]);
  if (
    asNumber(root["protocol_version"]) !== 1 ||
    asNumber(root["runtime_contract_version"]) !== 1 ||
    asNumber(root["snapshot_schema_version"]) !== 1
  ) {
    throw new ProtocolValidationError("unsupported_protocol_version");
  }
  asString(root["base_revision"]);
  asString(root["component"]);
  asString(root["correlation_id"]);
  asString(root["idempotency_key"]);
  asRecord(root["extensions"]);
  asRecord(root["model_proposals"]);
  const snapshot = asRecord(root["snapshot"]);
  const kind = asString(snapshot["kind"]);
  if (kind !== "instance" && kind !== "seed_promotion") {
    throw new ProtocolValidationError("invalid_snapshot_input_form");
  }
  if (kind === "seed_promotion") asString(snapshot["browser_nonce"]);
  asRecord(snapshot["envelope"]);
  let invoked = false;
  for (const raw of asArray(root["operations"])) {
    const operation = asRecord(raw);
    const operationKind = asString(operation["kind"]);
    if (operationKind === "sync_model" && !invoked) asString(operation["field"]);
    else if (operationKind === "invoke_action" && !invoked) {
      invoked = true;
      asString(operation["name"]);
      asRecord(operation["arguments"]);
    } else throw new ProtocolValidationError("incompatible_operation_batch");
  }
}

export function validateUpdateResponse(text: string): void {
  let root: Readonly<Record<string, unknown>>;
  try {
    root = asRecord(parseCanonicalJson(text));
  } catch (error: unknown) {
    const code = error instanceof Error ? error.message : "invalid_protocol_envelope";
    throw new ProtocolValidationError(code === "duplicate_key" ? "protocol_duplicate_field" : code);
  }
  requireExactKeys(
    root,
    [
      "correlation_id",
      "effects",
      "events",
      "extensions",
      "outcome",
      "protocol_version",
      "validation",
    ],
    ["accepted_revision", "error", "redirect", "render", "snapshot"],
  );
  if (asNumber(root["protocol_version"]) !== 1) {
    throw new ProtocolValidationError("unsupported_protocol_version");
  }
  asString(root["correlation_id"]);
  asArray(root["effects"]);
  asArray(root["events"]);
  asRecord(root["extensions"]);
  asRecord(root["validation"]);
  const outcome = asString(root["outcome"]);
  const redirect = root["redirect"];
  if (redirect !== undefined) {
    const target = asString(redirect);
    if (!target.startsWith("/") || target.startsWith("//") || target.includes("\\")) {
      throw new ProtocolValidationError("unsafe_redirect");
    }
    if (root["snapshot"] !== undefined || root["render"] !== undefined) {
      throw new ProtocolValidationError("response_outcome_mismatch");
    }
  } else if (
    (outcome === "accepted" || outcome === "duplicate") &&
    root["snapshot"] === undefined
  ) {
    throw new ProtocolValidationError("response_outcome_mismatch");
  }
}
