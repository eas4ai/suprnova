/// The limits one Live page runs under.
///
/// The server's configuration is the single source of every value here: the
/// bootstrap writes them into the inert configuration element and the runtime
/// reads them at startup (spec 09). The browser never enforces a limit tighter
/// than the server's, so `SERVER_DEFAULT_LIMITS` holds the server's own defaults
/// and serves only callers that run before or without that element, such as the
/// exported protocol validators.
export interface LiveLimits {
  /// One Live request body the browser sends (`LIVE_MAX_REQUEST_BYTES`).
  readonly maxRequestBytes: number;
  /// One Live response body the browser reads (`LIVE_MAX_RESPONSE_BYTES`).
  readonly maxResponseBytes: number;
  /// One island render's HTML, apart from the response around it
  /// (`LIVE_MAX_HTML_BYTES`).
  readonly maxHtmlBytes: number;
  /// Container nesting in Live JSON (`LIVE_MAX_JSON_DEPTH`). The parsers that
  /// walk it are recursive, so this is also their stack guard.
  readonly maxJsonDepth: number;
  /// Array elements plus object members in one request (`LIVE_MAX_JSON_ENTRIES`).
  readonly maxJsonEntries: number;
  /// Items in one request collection: model proposals, operations, action
  /// arguments, extensions (`LIVE_MAX_REQUEST_ITEMS`).
  readonly maxRequestItems: number;
  /// Items in one response collection: validation entries, events, effects,
  /// extensions, child deliveries (`LIVE_MAX_RESPONSE_ITEMS`).
  readonly maxResponseItems: number;
  /// Nodes in one rendered island the morph accepts (`LIVE_MORPH_MAX_NODES`).
  readonly morphMaxNodes: number;
  /// Element nesting in one rendered island (`LIVE_MORPH_MAX_DEPTH`). The morph
  /// library walks the tree recursively, so this guards the call stack.
  readonly morphMaxDepth: number;
  /// Keyed elements in one rendered island (`LIVE_MORPH_MAX_KEYS`).
  readonly morphMaxKeys: number;
  /// Attributes across one rendered island (`LIVE_MORPH_MAX_ATTRIBUTES`).
  readonly morphMaxAttributes: number;
  /// Attributes on one rendered element (`LIVE_MORPH_MAX_ATTRIBUTES_PER_ELEMENT`).
  /// Attribute reconciliation looks each name up in the element's list, so its
  /// cost grows with the square of this count.
  readonly morphMaxAttributesPerElement: number;
  /// Milliseconds one morph may run before it is abandoned
  /// (`LIVE_MORPH_DEADLINE_MS`); `0` means no deadline.
  readonly morphDeadlineMs: number;
  /// One redirect URL a Live response may carry (`LIVE_MAX_REDIRECT_BYTES`).
  readonly maxRedirectBytes: number;
  /// Asynchronous events the document may hold before its islands apply them
  /// (`LIVE_ASYNC_MAX_QUEUED_EVENTS`).
  readonly asyncMaxQueuedEvents: number;
  /// Events in one asynchronous replay after a reconnect
  /// (`LIVE_ASYNC_MAX_REPLAY_EVENTS`). A replay is queued whole, so it fits
  /// inside the queued-event limit.
  readonly asyncMaxReplayEvents: number;
  /// Bytes in one upload chunk request (`LIVE_UPLOAD_CHUNK_BYTES`).
  readonly uploadChunkBytes: number;
  /// Upload transfers running at once (`LIVE_UPLOAD_MAX_ACTIVE`).
  readonly uploadMaxActive: number;
  /// Bytes in one uploaded file (`LIVE_UPLOAD_MAX_FILE_BYTES`).
  readonly uploadMaxFileBytes: number;
  /// Bytes across the files one page has selected and not yet finished
  /// (`LIVE_UPLOAD_MAX_PENDING_BYTES`).
  readonly uploadMaxPendingBytes: number;
  /// Files one page has selected and not yet finished
  /// (`LIVE_UPLOAD_MAX_PENDING_FILES`).
  readonly uploadMaxPendingFiles: number;
}

const MIB = 1024 * 1024;
const GIB = 1024 * MIB;

/// The server's defaults, mirrored from `framework/src/live/config.rs`.
export const SERVER_DEFAULT_LIMITS: LiveLimits = Object.freeze({
  maxRequestBytes: 16 * MIB,
  maxResponseBytes: 16 * MIB,
  maxHtmlBytes: 16 * MIB,
  maxJsonDepth: 32,
  maxJsonEntries: 1_000_000,
  maxRequestItems: 65_536,
  maxResponseItems: 65_536,
  morphMaxNodes: 1_000_000,
  morphMaxDepth: 512,
  morphMaxKeys: 1_000_000,
  morphMaxAttributes: 10_000_000,
  morphMaxAttributesPerElement: 4_096,
  morphDeadlineMs: 0,
  maxRedirectBytes: 65_536,
  asyncMaxQueuedEvents: 4_096,
  asyncMaxReplayEvents: 4_096,
  uploadChunkBytes: 8 * MIB,
  uploadMaxActive: 8,
  uploadMaxFileBytes: GIB,
  uploadMaxPendingBytes: 4 * GIB,
  uploadMaxPendingFiles: 1_024,
});

/// The engine's hard ceiling on JSON nesting. A recursive walk that runs before
/// the configuration is known uses it, because no server configuration can
/// exceed it.
export const JSON_DEPTH_CEILING = 64;

export type LiveLimitName = keyof LiveLimits;

interface LimitDescriptor {
  readonly key: string;
  readonly label: string;
  readonly unit: string;
}

const DESCRIPTORS: Readonly<Record<LiveLimitName, LimitDescriptor>> = Object.freeze({
  maxRequestBytes: { key: "LIVE_MAX_REQUEST_BYTES", label: "request size", unit: "bytes" },
  maxResponseBytes: { key: "LIVE_MAX_RESPONSE_BYTES", label: "response size", unit: "bytes" },
  maxHtmlBytes: { key: "LIVE_MAX_HTML_BYTES", label: "island HTML size", unit: "bytes" },
  maxJsonDepth: { key: "LIVE_MAX_JSON_DEPTH", label: "JSON nesting depth", unit: "levels" },
  maxJsonEntries: {
    key: "LIVE_MAX_JSON_ENTRIES",
    label: "request JSON entry count",
    unit: "entries",
  },
  maxRequestItems: { key: "LIVE_MAX_REQUEST_ITEMS", label: "request item count", unit: "items" },
  maxResponseItems: { key: "LIVE_MAX_RESPONSE_ITEMS", label: "response item count", unit: "items" },
  morphMaxNodes: { key: "LIVE_MORPH_MAX_NODES", label: "morph node count", unit: "nodes" },
  morphMaxDepth: { key: "LIVE_MORPH_MAX_DEPTH", label: "morph nesting depth", unit: "levels" },
  morphMaxKeys: { key: "LIVE_MORPH_MAX_KEYS", label: "morph key count", unit: "keyed elements" },
  morphMaxAttributes: {
    key: "LIVE_MORPH_MAX_ATTRIBUTES",
    label: "morph attribute count",
    unit: "attributes",
  },
  morphMaxAttributesPerElement: {
    key: "LIVE_MORPH_MAX_ATTRIBUTES_PER_ELEMENT",
    label: "morph attributes-per-element count",
    unit: "attributes",
  },
  morphDeadlineMs: { key: "LIVE_MORPH_DEADLINE_MS", label: "morph deadline", unit: "ms" },
  maxRedirectBytes: { key: "LIVE_MAX_REDIRECT_BYTES", label: "redirect URL size", unit: "bytes" },
  asyncMaxQueuedEvents: {
    key: "LIVE_ASYNC_MAX_QUEUED_EVENTS",
    label: "async queued event count",
    unit: "events",
  },
  asyncMaxReplayEvents: {
    key: "LIVE_ASYNC_MAX_REPLAY_EVENTS",
    label: "async replay event count",
    unit: "events",
  },
  uploadChunkBytes: { key: "LIVE_UPLOAD_CHUNK_BYTES", label: "upload chunk size", unit: "bytes" },
  uploadMaxActive: {
    key: "LIVE_UPLOAD_MAX_ACTIVE",
    label: "upload transfer count",
    unit: "transfers",
  },
  uploadMaxFileBytes: {
    key: "LIVE_UPLOAD_MAX_FILE_BYTES",
    label: "upload file size",
    unit: "bytes",
  },
  uploadMaxPendingBytes: {
    key: "LIVE_UPLOAD_MAX_PENDING_BYTES",
    label: "upload pending size",
    unit: "bytes",
  },
  uploadMaxPendingFiles: {
    key: "LIVE_UPLOAD_MAX_PENDING_FILES",
    label: "upload pending file count",
    unit: "files",
  },
});

/// One limit that tripped, with everything a developer needs to change it.
///
/// A bare code such as `protocol_input_too_large` says that something was too
/// big but not what, by how much, or where to change it; this record carries
/// the limit, the measured and configured values, and the configuration key.
/// It holds only sizes and fixed names, never page content, so it is safe to
/// print.
export interface LiveLimitBreach {
  readonly limit: LiveLimitName;
  readonly key: string;
  readonly measured: number;
  readonly configured: number;
  /// The measurement stopped at the limit, so the real size is larger still.
  readonly atLeast: boolean;
  /// What was counted, when the limit covers several collections.
  readonly subject: string | null;
  readonly message: string;
}

export interface LimitBreachOptions {
  readonly atLeast?: boolean;
  readonly subject?: string;
}

export function limitBreach(
  limit: LiveLimitName,
  measured: number,
  configured: number,
  options: LimitBreachOptions = {},
): LiveLimitBreach {
  const descriptor = DESCRIPTORS[limit];
  const atLeast = options.atLeast === true;
  const subject = options.subject ?? null;
  const context = subject === null ? "" : ` (${subject})`;
  const message =
    `Suprnova Live ${descriptor.label} limit exceeded${context}: measured ` +
    `${atLeast ? "at least " : ""}${String(measured)} ${descriptor.unit}, configured ` +
    `${String(configured)} ${descriptor.unit}. Raise ${descriptor.key} in the ` +
    `application's .env file to allow it.`;
  return Object.freeze({
    atLeast,
    configured,
    key: descriptor.key,
    limit,
    measured,
    message,
    subject,
  });
}

/// An error raised because a configured limit tripped. Its message is the
/// breach's readable sentence followed by the stable machine code, so a test or
/// a log search can still match the code.
export class LiveLimitError extends Error {
  readonly liveLimit: LiveLimitBreach;

  constructor(
    readonly code: string,
    breach: LiveLimitBreach,
  ) {
    super(`${breach.message} (${code})`);
    this.name = "LiveLimitError";
    this.liveLimit = breach;
  }
}

/// The breach an error carries, from any of the error families that can report
/// one: protocol validation, morph parsing, transport reads.
export function breachOf(error: unknown): LiveLimitBreach | null {
  if (error === null || typeof error !== "object") return null;
  const breach: unknown = Reflect.get(error, "liveLimit");
  return breach !== null && typeof breach === "object" && "key" in breach
    ? (breach as LiveLimitBreach)
    : null;
}

/// Validates one limit set: every value a positive safe integer, the deadline
/// a non-negative one, and the pairs that must nest nested.
export function validLiveLimits(limits: LiveLimits): boolean {
  const positive = (Object.keys(SERVER_DEFAULT_LIMITS) as LiveLimitName[])
    .filter((name) => name !== "morphDeadlineMs")
    .every((name) => Number.isSafeInteger(limits[name]) && limits[name] > 0);
  return (
    positive &&
    Number.isSafeInteger(limits.morphDeadlineMs) &&
    limits.morphDeadlineMs >= 0 &&
    limits.maxJsonDepth <= JSON_DEPTH_CEILING &&
    limits.morphMaxAttributesPerElement <= limits.morphMaxAttributes &&
    limits.maxHtmlBytes <= limits.maxResponseBytes &&
    limits.maxResponseBytes <= limits.maxRequestBytes &&
    limits.maxRedirectBytes <= limits.maxResponseBytes &&
    limits.asyncMaxReplayEvents <= limits.asyncMaxQueuedEvents &&
    limits.uploadChunkBytes <= limits.uploadMaxFileBytes &&
    limits.uploadMaxFileBytes <= limits.uploadMaxPendingBytes &&
    limits.uploadMaxActive <= limits.uploadMaxPendingFiles
  );
}

export function utf8Length(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}
