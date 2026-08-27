const DOCUMENT_BYTES = 65_536 as const;
const D100_ISLANDS = 100;
const SIGNATURE = "A".repeat(43);
const RUNTIME_CONFIG = `<script id="suprnova-live-config" type="application/json">${JSON.stringify({
  asset_identity: "browser-budget-v1",
  credentials: "same-origin",
  endpoint: "/live",
  max_parallel_per_island: 1,
  max_queued_per_island: 8,
  max_response_bytes: 1_048_576,
  protocol: { maximum: 2, minimum: 1 },
  request_timeout_ms: 5_000,
  runtime_contract_version: 1,
})}</script>`;

export type MorphWorkloadId = "M1K" | "M5K";

export interface E100Workload {
  readonly id: "E100/1K";
  readonly subscriptionCount: 100;
  readonly presentationEventCount: 1_000;
  readonly eventEnvelopeBytes: 1_024;
  readonly scheduledDurationMs: 10_000;
  readonly refreshInvalidationCount: 100;
}

export interface R100Workload {
  readonly id: "R100";
  readonly subscriptionCount: 100;
  readonly simultaneousContinuityLosses: 100;
  readonly multiDocumentCount: 16;
}

export interface D100Workload {
  readonly id: "D100";
  readonly html: string;
  readonly documentBytes: 65_536;
  readonly islandCount: 100;
}

export interface MorphWorkload {
  readonly id: MorphWorkloadId;
  readonly sourceDocument: string;
  readonly sourceHtml: string;
  readonly targetHtml: string;
  readonly targetSnapshot: Readonly<Record<string, unknown>>;
  readonly targetAuthority: Readonly<{
    component: "c";
    documentKey: string;
    encodedSnapshot: string;
    instanceId: string;
    slot: string;
    successorRevision: "8";
  }>;
  readonly elementCount: number;
  readonly maximumDepth: number;
  readonly changedNodeCount: number;
}

function binaryIdentity(index: number): string {
  const bytes = Buffer.alloc(16);
  bytes.writeUInt32BE(index, 12);
  return bytes.toString("base64url");
}

function snapshot(instanceId: string | null, slot: string, revision: number) {
  return {
    body:
      instanceId === null
        ? { component: { name: "c" }, form: "seed", slot }
        : {
            build_id: "browser-budget-v1",
            component: {
              contract_digest: "ICEiIyQlJicoKSorLC0uLzAxMjM0NTY3ODk6Ozw9Pj8",
              memo_schema_version: 1,
              mount_schema_version: 1,
              name: "c",
              state_schema_version: 1,
            },
            expires_at: "4102444800000",
            extensions: {},
            form: "instance",
            instance_id: instanceId,
            issued_at: "0",
            key_id: "browser-budget-v1",
            memo: {},
            revision: String(revision),
            route: "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA",
            schema_version: 1,
            scope: "kJGSk5SVlpeYmZqbnJ2en6ChoqOkpaanqKmqq6ytrq8",
            slot,
            state: {},
          },
    signature: SIGNATURE,
  } as const;
}

function encodeSnapshot(value: Readonly<Record<string, unknown>>): string {
  return Buffer.from(JSON.stringify(value), "utf8").toString("base64url");
}

function islandRoot(
  body: string,
  documentKey: string,
  instanceId: string | null,
  slot: string,
  revision: number,
): { readonly html: string; readonly snapshot: Readonly<Record<string, unknown>> } {
  const envelope = snapshot(instanceId, slot, revision);
  const form = instanceId === null ? "seed" : "instance";
  const instance = instanceId === null ? "" : ` data-suprnova-live-instance="${instanceId}"`;
  return {
    html: `<section data-suprnova-live-root="${slot}" data-suprnova-live-island data-suprnova-live-component="c" data-suprnova-live-slot="${slot}" data-suprnova-live-document-key="${documentKey}" data-suprnova-live-protocol-min="2" data-suprnova-live-contract="1" data-suprnova-live-snapshot-kind="${form}" data-suprnova-live-snapshot="${encodeSnapshot(envelope)}" data-suprnova-live-revision="${String(revision)}" data-suprnova-live-lazy-complete="false"${instance}>${body}</section>`,
    snapshot: envelope,
  };
}

function documentWith(body: string): string {
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Suprnova Live browser budget</title>${RUNTIME_CONFIG}</head><body>${body}</body></html>`;
}

export function createD100Workload(): D100Workload {
  const islands = Array.from({ length: D100_ISLANDS }, (_, index) => {
    const slot = "s";
    return islandRoot("", `d${index.toString(36)}`, null, slot, 0).html;
  }).join("");
  const opening = `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Suprnova Live D100</title>${RUNTIME_CONFIG}</head><body>`;
  const closing = "</body></html>";
  const unpaddedBytes = Buffer.byteLength(opening + islands + closing, "utf8");
  const commentUnits = DOCUMENT_BYTES - unpaddedBytes;
  if (commentUnits < 7) throw new Error(`d100_document_overflow:${String(unpaddedBytes)}`);
  const html = `${opening}${islands}<!--${"x".repeat(commentUnits - 7)}-->${closing}`;
  if (Buffer.byteLength(html, "utf8") !== DOCUMENT_BYTES) {
    throw new Error("d100_document_size");
  }
  return Object.freeze({
    id: "D100",
    html,
    documentBytes: DOCUMENT_BYTES,
    islandCount: D100_ISLANDS,
  });
}

export function createE100Workload(): E100Workload {
  return Object.freeze({
    id: "E100/1K" as const,
    subscriptionCount: 100 as const,
    presentationEventCount: 1_000 as const,
    eventEnvelopeBytes: 1_024 as const,
    scheduledDurationMs: 10_000 as const,
    refreshInvalidationCount: 100 as const,
  });
}

export function createR100Workload(): R100Workload {
  return Object.freeze({
    id: "R100" as const,
    subscriptionCount: 100 as const,
    simultaneousContinuityLosses: 100 as const,
    multiDocumentCount: 16 as const,
  });
}

function morphShape(id: MorphWorkloadId) {
  return id === "M1K"
    ? { elementCount: 1_000, maximumDepth: 12 }
    : { elementCount: 5_000, maximumDepth: 24 };
}

function keyedElement(index: number, changed: boolean, child = ""): string {
  const value = changed ? "changed" : "stable";
  if (child.length > 0) {
    return `<div data-suprnova-live-key="node-${String(index)}" data-budget-value="${value}">${child}</div>`;
  }
  return `<span data-suprnova-live-key="node-${String(index)}" data-budget-value="${value}">Node ${String(index)}${changed ? " changed" : ""}</span>`;
}

function keyedTree(elementCount: number, maximumDepth: number, target: boolean): string {
  let chain = "";
  for (let index = maximumDepth - 1; index >= 0; index -= 1) {
    chain = keyedElement(index, target && index % 10 === 0, chain);
  }
  const balancedCount = elementCount - maximumDepth;
  const children = Array.from({ length: balancedCount }, () => [] as number[]);
  for (let relative = 1; relative < balancedCount; relative += 1) {
    const parent = Math.floor((relative - 1) / 8);
    children[parent]?.push(relative);
  }
  const renderBalanced = (relative: number): string => {
    const index = maximumDepth + relative;
    const descendants = children[relative]?.map(renderBalanced).join("") ?? "";
    return keyedElement(index, target && index % 10 === 0, descendants);
  };
  return `${chain}${balancedCount === 0 ? "" : renderBalanced(0)}`;
}

export function createMorphWorkload(id: MorphWorkloadId): MorphWorkload {
  const { elementCount, maximumDepth } = morphShape(id);
  const changedNodeCount = elementCount / 10;
  const instanceId = binaryIdentity(id === "M1K" ? 1_001 : 5_001);
  const slot = id.toLowerCase();
  const action = `<button type="button" id="${slot}-action" live:click.prevent="measure">Measure ${id}</button>`;
  const source = islandRoot(
    `${action}${keyedTree(elementCount, maximumDepth, false)}`,
    slot,
    instanceId,
    slot,
    7,
  );
  const target = islandRoot(
    `${action}${keyedTree(elementCount, maximumDepth, true)}`,
    slot,
    instanceId,
    slot,
    8,
  );
  return Object.freeze({
    id,
    sourceDocument: documentWith(source.html),
    sourceHtml: source.html,
    targetHtml: target.html,
    targetSnapshot: target.snapshot,
    targetAuthority: Object.freeze({
      component: "c" as const,
      documentKey: slot,
      encodedSnapshot: encodeSnapshot(target.snapshot),
      instanceId,
      slot,
      successorRevision: "8" as const,
    }),
    elementCount,
    maximumDepth,
    changedNodeCount,
  });
}
