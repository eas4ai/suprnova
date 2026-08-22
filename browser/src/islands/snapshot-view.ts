import { parseCanonicalJson } from "../canonical.js";

const MAX_SNAPSHOT_BYTES = 131_072;
const MAX_UNSIGNED_64 = 18_446_744_073_709_551_615n;
const SNAPSHOT_LIMITS = Object.freeze({
  maxBytes: MAX_SNAPSHOT_BYTES,
  maxDepth: 32,
  maxEntries: 4_096,
  maxStringBytes: MAX_SNAPSHOT_BYTES,
});

export type SnapshotForm = "seed" | "instance";

export interface SnapshotPublicView {
  readonly envelope: Readonly<Record<string, unknown>>;
  readonly form: SnapshotForm;
  readonly component: string;
  readonly slot: string;
  readonly instanceId: string | null;
  readonly revision: bigint;
}

export class SnapshotViewError extends Error {
  constructor(readonly code: "encoding" | "shape" | "limit") {
    super(`snapshot_view_${code}`);
    this.name = "SnapshotViewError";
  }
}

function record(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function exactKeys(value: Record<string, unknown>, expected: readonly string[]): boolean {
  const keys = Object.keys(value).sort();
  return keys.length === expected.length && keys.every((key, index) => key === expected[index]);
}

function decimal(value: unknown): bigint | null {
  if (
    typeof value !== "string" ||
    value.length > 20 ||
    !/^(0|[1-9][0-9]*)$/u.test(value)
  ) {
    return null;
  }
  const parsed = BigInt(value);
  return parsed <= MAX_UNSIGNED_64 ? parsed : null;
}

function decode(encoded: string): string {
  if (
    encoded.length === 0 ||
    encoded.length > Math.ceil((MAX_SNAPSHOT_BYTES * 4) / 3) ||
    !/^[A-Za-z0-9_-]+$/u.test(encoded)
  ) {
    throw new SnapshotViewError("encoding");
  }
  const padding = "=".repeat((4 - (encoded.length % 4)) % 4);
  let binary: string;
  try {
    binary = globalThis.atob(encoded.replace(/-/gu, "+").replace(/_/gu, "/") + padding);
  } catch {
    throw new SnapshotViewError("encoding");
  }
  if (binary.length > MAX_SNAPSHOT_BYTES) throw new SnapshotViewError("limit");
  const bytes = Uint8Array.from(binary, (unit) => unit.codePointAt(0) ?? 0);
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    throw new SnapshotViewError("encoding");
  }
}

function publicComponent(value: unknown): string | null {
  const component = record(value);
  return component === null || typeof component["name"] !== "string" ? null : component["name"];
}

export function decodeSnapshotPublicView(encoded: string): SnapshotPublicView {
  let parsed: unknown;
  try {
    parsed = parseCanonicalJson(decode(encoded), SNAPSHOT_LIMITS);
  } catch (error: unknown) {
    if (error instanceof SnapshotViewError) throw error;
    throw new SnapshotViewError("shape");
  }
  const envelope = record(parsed);
  if (envelope === null || !exactKeys(envelope, ["body", "signature"])) {
    throw new SnapshotViewError("shape");
  }
  if (
    typeof envelope["signature"] !== "string" ||
    !/^[A-Za-z0-9_-]{43}$/u.test(envelope["signature"])
  ) {
    throw new SnapshotViewError("shape");
  }
  const body = record(envelope["body"]);
  const component = body === null ? null : publicComponent(body["component"]);
  const slot = body?.["slot"];
  const form = body?.["form"];
  if (
    body === null ||
    component === null ||
    typeof slot !== "string" ||
    (form !== "seed" && form !== "instance")
  ) {
    throw new SnapshotViewError("shape");
  }
  if (form === "seed") {
    if (body["instance_id"] !== undefined || body["revision"] !== undefined) {
      throw new SnapshotViewError("shape");
    }
    return Object.freeze({
      envelope: Object.freeze(envelope),
      form,
      component,
      slot,
      instanceId: null,
      revision: 0n,
    });
  }
  const instanceId = body["instance_id"];
  const revision = decimal(body["revision"]);
  if (typeof instanceId !== "string" || revision === null) {
    throw new SnapshotViewError("shape");
  }
  return Object.freeze({
    envelope: Object.freeze(envelope),
    form,
    component,
    slot,
    instanceId,
    revision,
  });
}
