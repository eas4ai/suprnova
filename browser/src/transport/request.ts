import { canonicalize, type JsonValue } from "../canonical.js";
import { validateUpdateRequest } from "../protocol.js";
import type { RuntimeRandomness } from "../runtime/ports.js";
import type { ServerIntent, ServerOperation } from "../scheduler/intent.js";

export const DOCUMENT_KEY_EXTENSION = "x_suprnova_live_document_key_v1";

const IDENTITY_BYTES = 16;
const MAX_EXTENSION_ENTRIES = 8;

export interface RequestIdentity {
  readonly correlationId: string;
  readonly idempotencyKey: string;
  readonly baseRevision: bigint;
  readonly semanticDigest: string;
  readonly promotionNonce: string | null;
}

export interface BuiltLiveRequest {
  readonly identity: RequestIdentity;
  readonly protocolVersion: 1 | 2;
  readonly mediaType: string;
  readonly text: string;
}

export interface LiveRequestBuildInput {
  readonly intent: ServerIntent;
  readonly protocolVersion: 1 | 2;
  readonly randomness: RuntimeRandomness;
  readonly childParameters?: Readonly<Record<string, JsonValue>>;
  readonly extensions?: Readonly<Record<string, JsonValue>>;
}

interface PriorBuild {
  readonly semanticDigest: string;
  readonly request: BuiltLiveRequest;
}

function base64Url(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCodePoint(byte);
  return globalThis.btoa(binary).replace(/\+/gu, "-").replace(/\//gu, "_").replace(/=+$/u, "");
}

function randomIdentity(randomness: RuntimeRandomness): string {
  const bytes = randomness.randomBytes(IDENTITY_BYTES);
  if (!(bytes instanceof Uint8Array) || bytes.byteLength !== IDENTITY_BYTES) {
    throw new Error("request_identity_unavailable");
  }
  return base64Url(new Uint8Array(bytes));
}

async function semanticDigest(value: JsonValue): Promise<string> {
  const digest = await globalThis.crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(canonicalize(value)),
  );
  return base64Url(new Uint8Array(digest));
}

function jsonOperation(operation: ServerOperation): JsonValue {
  switch (operation.kind) {
    case "sync_model":
      return { field: operation.field, kind: operation.kind };
    case "invoke_action":
      return { arguments: operation.arguments, kind: operation.kind, name: operation.name };
    case "params_changed":
    case "lazy_complete":
    case "fresh_render":
      return { kind: operation.kind };
  }
}

function requestExtensions(input: LiveRequestBuildInput): Readonly<Record<string, JsonValue>> {
  const extensionEntries = Object.entries(input.extensions ?? {});
  if (extensionEntries.some(([name]) => name === DOCUMENT_KEY_EXTENSION)) {
    throw new Error("request_document_key_conflict");
  }
  if (extensionEntries.length + 1 > MAX_EXTENSION_ENTRIES) {
    throw new Error("request_extension_limit");
  }
  return Object.freeze({
    ...Object.fromEntries(extensionEntries),
    [DOCUMENT_KEY_EXTENSION]: input.intent.source.island.metadata.documentKey,
  });
}

function snapshotInput(intent: ServerIntent): Readonly<Record<string, JsonValue>> {
  const metadata = intent.source.island.metadata;
  const envelope = metadata.snapshot as Readonly<Record<string, JsonValue>>;
  if (metadata.snapshotForm === "instance") {
    if (intent.promotionNonce() !== null) throw new Error("request_snapshot_form");
    return Object.freeze({ envelope, kind: "instance" });
  }
  const browserNonce = intent.promotionNonce();
  if (browserNonce === null) throw new Error("request_snapshot_form");
  return Object.freeze({ browser_nonce: browserNonce, envelope, kind: "seed_promotion" });
}

function envelopeWithoutIdentity(
  input: LiveRequestBuildInput,
  extensions: Readonly<Record<string, JsonValue>>,
): Readonly<Record<string, JsonValue>> {
  const metadata = input.intent.source.island.metadata;
  const operations = Object.freeze(input.intent.operations.map(jsonOperation));
  if (
    input.protocolVersion === 1 &&
    input.intent.operations.some(
      (operation) =>
        operation.kind === "params_changed" ||
        operation.kind === "lazy_complete" ||
        operation.kind === "fresh_render",
    )
  ) {
    throw new Error("request_protocol_form");
  }
  if (input.protocolVersion === 1 && input.childParameters !== undefined) {
    throw new Error("request_protocol_form");
  }
  const common = {
    base_revision: metadata.revision.toString(10),
    component: metadata.component,
    extensions,
    model_proposals: input.intent.modelProposals,
    operations,
    protocol_version: input.protocolVersion,
    runtime_contract_version: input.protocolVersion,
    snapshot: snapshotInput(input.intent),
    snapshot_schema_version: 1,
  } satisfies Readonly<Record<string, JsonValue>>;
  if (input.protocolVersion === 1) return Object.freeze(common);
  return Object.freeze({
    ...common,
    child_parameters: input.childParameters ?? input.intent.childParameters ?? null,
  });
}

function mediaType(version: 1 | 2): string {
  return `application/vnd.suprnova.live+json; charset=utf-8; version=${String(version)}`;
}

export class LiveRequestBuilder {
  readonly #prior = new WeakMap<ServerIntent, PriorBuild>();

  async build(input: LiveRequestBuildInput): Promise<BuiltLiveRequest> {
    const extensions = requestExtensions(input);
    const semantic = envelopeWithoutIdentity(input, extensions);
    let digest: string;
    try {
      digest = await semanticDigest(semantic);
    } catch {
      input.intent.finish("terminal");
      throw new Error("request_identity_unavailable");
    }
    const prior = this.#prior.get(input.intent);
    if (prior?.semanticDigest === digest) return prior.request;

    let correlationId: string;
    let idempotencyKey: string;
    try {
      correlationId = randomIdentity(input.randomness);
      idempotencyKey = randomIdentity(input.randomness);
    } catch {
      input.intent.finish("terminal");
      throw new Error("request_identity_unavailable");
    }
    const value = {
      ...semantic,
      correlation_id: correlationId,
      idempotency_key: idempotencyKey,
    } as JsonValue;
    const text = canonicalize(value);
    validateUpdateRequest(text);
    const identity = Object.freeze({
      baseRevision: input.intent.source.island.metadata.revision,
      correlationId,
      idempotencyKey,
      promotionNonce: input.intent.promotionNonce(),
      semanticDigest: digest,
    });
    const request = Object.freeze({
      identity,
      mediaType: mediaType(input.protocolVersion),
      protocolVersion: input.protocolVersion,
      text,
    });
    this.#prior.set(input.intent, Object.freeze({ request, semanticDigest: digest }));
    return request;
  }
}
