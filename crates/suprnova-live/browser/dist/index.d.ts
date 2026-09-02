declare module "@suprnova/live" {
type DiagnosticMode = "off" | "errors" | "verbose";
export type RuntimeStatus = "running" | "suspended" | "stopped";
export type JsonValue = null | boolean | number | string | JsonArray | JsonObject;
interface JsonArray extends ReadonlyArray<JsonValue> {
  readonly [index: number]: JsonValue;
}
interface JsonObject {
  readonly [key: string]: JsonValue;
}
export type PayloadSchema =
  | Readonly<{ type: "null" }>
  | Readonly<{ type: "boolean" }>
  | Readonly<{ type: "number" }>
  | Readonly<{ type: "integer" }>
  | Readonly<{ type: "string"; maxBytes?: number }>
  | Readonly<{ type: "array"; items: PayloadSchema; maxItems: number }>
  | Readonly<{
      type: "object";
      properties: Readonly<Record<string, PayloadSchema>>;
      required: readonly string[];
      additionalProperties: false;
    }>;
interface IslandExtensionIdentity {
  readonly component: string;
  readonly slot: string;
  readonly documentKey: string;
}
export interface EffectContext {
  readonly island: IslandExtensionIdentity;
  call(name: string, input: JsonValue): Promise<JsonValue>;
}
export interface EffectRegistration {
  readonly name: string;
  readonly version: number;
  readonly schema: PayloadSchema;
  readonly phase: "after_commit";
  run(context: EffectContext, payload: JsonValue): void | Promise<void>;
}
export interface RuntimeCallContext {
  readonly island: IslandExtensionIdentity;
  server(name: string, input: JsonValue): Promise<JsonValue>;
  local(name: string, input: JsonValue): Promise<JsonValue>;
}
export interface RuntimeCallRegistration {
  readonly name: string;
  readonly input: PayloadSchema;
  readonly output: PayloadSchema;
  run(context: RuntimeCallContext, input: JsonValue): JsonValue | Promise<JsonValue>;
}
export interface StimulusApplicationPort {
  start(): void;
  stop(): void;
  load(...definitions: readonly unknown[]): void;
  unload(...identifiers: readonly string[]): void;
}
export interface StimulusBootstrapOptions {
  readonly application: StimulusApplicationPort;
  readonly definitions?: readonly unknown[];
}
export interface StimulusContinuityRoot {
  readonly identity: string;
  readonly element: Element;
}
export interface StimulusContinuity {
  readonly scope: Element;
  readonly scopeIdentity: string | null;
  readonly roots: readonly StimulusContinuityRoot[];
}
export interface StimulusMorphBridge {
  beforeMorph(scope: Element): StimulusContinuity;
  afterMorph(continuity: StimulusContinuity, scope: Element): void;
  disposeScope(scope: Element): void;
  dispose(): void;
}
export type RuntimeFeatureRegistrationOutcome =
  | "registered"
  | "already_registered"
  | "incompatible"
  | "conflict"
  | "registry_full";
export type RuntimeFeature = readonly [
  format: symbol,
  slot: 0 | 1,
  capabilityVersion: 1,
  packedCoreRange: number,
  identity: object,
  drive: (...arguments_: readonly unknown[]) => boolean,
];
type RuntimeFeatureDiagnosticDetail =
  | "contract_mismatch"
  | "operation_rejected"
  | "resource_exhausted";
export type FreshRenderReason = "poll" | "stream";
  export type FreshRenderDisposition = "queued" | "coalesced" | "retired" | "exhausted";
export type FreshRenderCompletion = "succeeded" | "failed" | "canceled" | "retired";
export type FreshRenderCompletionObserver = (completion: FreshRenderCompletion) => void;
  interface PartiallyDispatchedBrowserEvent {
    readonly delivered: number;
    readonly kind: "partially_dispatched";
    readonly reason:
      | "capability_rotated"
      | "dispatch_failed"
      | "source_retired"
      | "target_retired";
    readonly skipped: number;
  }
  type RegisteredBrowserEventDisposition =
    | "dispatched"
    | "no_target"
    | "fanout_exceeded"
    | "rejected"
    | "retired"
    | PartiallyDispatchedBrowserEvent;
const REGISTERED_BROWSER_EVENT_CAPABILITY: unique symbol;
interface RegisteredBrowserEventCapability {
  readonly [REGISTERED_BROWSER_EVENT_CAPABILITY]: never;
}
interface RegisteredBrowserEventDispatch {
  readonly event: string;
  readonly payload: JsonValue;
  readonly schemaVersion: number;
  readonly target: string;
}
interface RegisteredBrowserEventContract {
  readonly cycle:
    | Readonly<{ kind: "forbid_repeated_island" }>
    | Readonly<{ kind: "maximum_hops"; maximumHops: number }>;
  readonly maximumFanout: number;
  readonly name: string;
  readonly order: "per_source_sequence";
  readonly payloadContract: string;
  readonly schema: "json" | "null" | "boolean" | "i64" | "u64" | "f64" | "string";
  readonly source: "stream";
  readonly targets: readonly string[];
  readonly version: number;
}
export type UploadHandle = string;
export type UploadHandleProposal = UploadHandle | readonly UploadHandle[] | null;
export type UploadHandleProposalDisposition = "accepted" | "unchanged" | "retired";
type FeatureDirectiveDiagnosticCode =
  | "not_live_directive"
  | "attribute_limit"
  | "unknown_directive"
  | "reserved_directive"
  | "invalid_modifier"
  | "repeated_modifier"
  | "invalid_value"
  | "unsafe_target"
  | "directive_conflict"
  | "dynamic_structure_unproved"
  | "unsupported_modifier"
  | "modifier_conflict";
interface ParsedFeatureDirective {
  readonly ok: true;
  readonly name: string;
  readonly value: string;
  readonly role: string | null;
  readonly modifiers: readonly string[];
  readonly capability: "uploads@1" | "async@1";
}
interface FeatureDirectiveDiagnostic {
  readonly ok: false;
  readonly code: FeatureDirectiveDiagnosticCode;
  readonly fallback: "inert" | "native" | "retain_dom";
}
type FeatureDirectiveParseResult = ParsedFeatureDirective | FeatureDirectiveDiagnostic;
type RuntimeFeatureDirectiveParser = (
  attributeName: string,
  value: string,
  presentDirectiveNames?: readonly string[],
) => FeatureDirectiveParseResult;
interface RuntimeFeatureDirectiveOwnership {
  readonly attributeName: string;
  readonly directive: ParsedFeatureDirective;
  readonly element: Element;
}
export interface RuntimeFeatureDocumentContext {
  diagnose(detail: RuntimeFeatureDiagnosticDetail): void;
  onDispose(dispose: () => void): void;
}
export interface RuntimeFeatureIslandPortBase {
  readonly element: Element;
  readonly identity: IslandExtensionIdentity;
  onDispose(dispose: () => void): void;
  queryDirectiveOwnership(
    parser: RuntimeFeatureDirectiveParser,
  ): readonly RuntimeFeatureDirectiveOwnership[];
}
const VALIDATED_ASYNC_DESCRIPTOR_CAPABILITY: unique symbol;
interface ValidatedAsyncDescriptorCapability {
  readonly [VALIDATED_ASYNC_DESCRIPTOR_CAPABILITY]: never;
}
export interface AsyncRuntimeIslandPort extends RuntimeFeatureIslandPortBase {
  consumeRegisteredEventCapability(
    descriptor: ValidatedAsyncDescriptorCapability,
  ): RegisteredBrowserEventCapability;
  dispatchRegisteredEvent(
    capability: RegisteredBrowserEventCapability,
    event: RegisteredBrowserEventDispatch,
  ): RegisteredBrowserEventDisposition;
  enqueueFreshRender(
      reason: FreshRenderReason,
      completion?: FreshRenderCompletionObserver,
      completionKey?: string,
  ): FreshRenderDisposition;
  writePresentationSignal(scope: string, name: string, value: JsonValue): JsonValue;
}
export interface UploadsRuntimeIslandPort extends RuntimeFeatureIslandPortBase {
  proposeUploadHandle(
    field: string,
    proposal: UploadHandleProposal,
  ): UploadHandleProposalDisposition;
}
export type RuntimeFeatureIslandPort = AsyncRuntimeIslandPort | UploadsRuntimeIslandPort;
export interface FeatureIslandController {
  abortMorph?(): void;
  afterMorph?(): void;
  beforeMorph?(): void;
  dispose(): void;
  resume?(): void;
  suspend?(): void;
}
export interface FeatureDocumentController<
  Port extends RuntimeFeatureIslandPort = RuntimeFeatureIslandPort,
> {
  connectIsland(port: Port): FeatureIslandController | undefined;
  dispose(): void;
  resume?(): void;
  suspend?(): void;
}
export const CLASSIC_FEATURE_SYMBOL: unique symbol;
export interface ClassicFeatureSurface {
  readonly version: 1;
  configureAsync(options: import("@suprnova/live/async").AsyncFeatureOptions): void;
  register(feature: unknown): RuntimeFeatureRegistrationOutcome;
}
export interface EffectInvocation {
  readonly name: string;
  readonly version?: number;
  readonly payload: unknown;
}
type EffectRunStatus =
  | "completed"
  | "missing"
  | "invalid"
  | "invalid_context"
  | "failed"
  | "timeout"
  | "canceled";
export interface EffectRunOutcome {
  readonly name: string;
  readonly version?: number;
  readonly status: EffectRunStatus;
}
export interface RuntimeHandle {
  status(): RuntimeStatus;
  stop(): void;
  runEffect(owner: Element, invocation: EffectInvocation): Promise<EffectRunOutcome>;
  call(owner: Element, name: string, input: JsonValue): Promise<JsonValue>;
}
export interface RuntimeClock { now(): number; }
export interface RuntimeRandomness { randomBytes(length: number): Uint8Array; }
export interface RuntimeConnectivity { isOnline(): boolean; }
export interface TransportPort {
  fetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response>;
}
export interface NavigationPort {
  assign(target: URL): void;
  replace(target: URL): void;
  reload(): void;
}
export interface RuntimeObserverFactory {
  mutation(callback: MutationCallback): MutationObserver;
  intersection(
    callback: IntersectionObserverCallback,
    options?: IntersectionObserverInit,
  ): IntersectionObserver | null;
}
export interface RuntimeScheduler {
  microtask(callback: VoidFunction): void;
  animationFrame(callback: FrameRequestCallback): number;
  cancelAnimationFrame(handle: number): void;
  timeout(callback: VoidFunction, milliseconds: number): number;
  clearTimeout(handle: number): void;
}
export interface RuntimeFeatures {
  prefersReducedMotion(): boolean;
  supportsViewTransitions(): boolean;
  supportsSpeculationRules(): boolean;
}
export interface RuntimePortOverrides {
  readonly clock?: RuntimeClock;
  readonly connectivity?: RuntimeConnectivity;
  readonly randomness?: RuntimeRandomness;
  readonly transport?: TransportPort;
  readonly navigation?: NavigationPort;
  readonly observers?: RuntimeObserverFactory;
  readonly scheduler?: RuntimeScheduler;
  readonly features?: RuntimeFeatures;
}
export interface BootstrapOptions extends RuntimePortOverrides {
  readonly document?: Document;
  readonly allowedEndpointOrigins?: readonly string[];
  readonly diagnostics?: DiagnosticMode;
  readonly effects?: readonly EffectRegistration[];
  readonly calls?: readonly RuntimeCallRegistration[];
  readonly extensionDeadlineMs?: number;
  readonly stimulus?: StimulusBootstrapOptions;
}
export interface RuntimeAsset {
  readonly file: string;
  readonly role: RuntimeAssetRole;
  readonly bytes: number;
  readonly sha256: string;
  readonly sri: `sha256-${string}`;
  readonly capability: RuntimeAssetCapability;
  readonly capability_version: 1;
  readonly compatible_core: ">=0.1.0 <0.2.0";
  readonly content_type: "text/javascript; charset=utf-8";
  readonly script_kind: "module" | "classic";
  readonly preload_rel: "modulepreload" | "preload";
  readonly cache_control: "public, max-age=31536000, immutable";
}
export type RuntimeAssetRole =
  | "core-esm"
  | "core-classic"
  | "stimulus-esm"
  | "stimulus-classic"
  | "uploads-esm"
  | "uploads-classic"
  | "async-esm"
  | "async-classic";
export type RuntimeAssetCapability = "core@1" | "stimulus@1" | "uploads@1" | "async@1";
export interface RuntimeAssetManifest {
  readonly schema_version: 2;
  readonly engine_version: "0.1.0";
  readonly runtime_contract_version: 1;
  readonly protocol_versions: readonly [1, 2];
  readonly snapshot_versions: readonly [1];
  readonly built_at: "1970-01-01T00:00:00.000Z";
  readonly assets: readonly RuntimeAsset[];
  readonly provenance: {
    readonly idiomorph: {
      readonly name: "idiomorph";
      readonly version: "0.7.4";
      readonly license: "0BSD";
      readonly bundled: true;
    };
  };
}
export interface SuprnovaLivePublicApi {
  readonly version: "0.1.0";
  readonly runtimeContractVersion: 1;
  readonly supportedProtocolVersions: readonly [1, 2];
  boot(options?: BootstrapOptions): RuntimeHandle;
}
export const version: "0.1.0";
export const runtimeContractVersion: 1;
export const supportedProtocolVersions: readonly [1, 2];
export const RUNTIME_SYMBOL: symbol;
export function boot(options?: BootstrapOptions): RuntimeHandle;
const api: SuprnovaLivePublicApi;
export default api;
}

declare module "@suprnova/live/runtime" {
export * from "@suprnova/live";
export { default } from "@suprnova/live";
}

declare module "@suprnova/live/stimulus" {
import type { RuntimeFeatureRegistrationOutcome } from "@suprnova/live";
export const stimulusRegistration: RuntimeFeatureRegistrationOutcome;
export function installStimulusAdapter(
  target?: typeof globalThis,
): RuntimeFeatureRegistrationOutcome;
export default stimulusRegistration;
}

declare module "@suprnova/live/uploads" {
import type {
  RuntimeFeature,
  RuntimeFeatureRegistrationOutcome,
  UploadHandle,
} from "@suprnova/live";
export type { UploadHandle } from "@suprnova/live";
export type UploadPresentationState =
  | "queued"
  | "transferring"
  | "verifying"
  | "ready"
  | "finalizing"
  | "finalized"
  | "interrupted"
  | "failed"
  | "canceled"
  | "expired";
export interface UploadFileIdentity {
  readonly lastModified: number;
  readonly name: string;
  readonly size: number;
  readonly type: string;
}
interface UploadRequestBase { readonly signal: AbortSignal; }
export interface CreateUploadRequest extends UploadRequestBase {
  readonly operation: "create";
  readonly field: string;
  readonly file: UploadFileIdentity;
  readonly idempotencyKey: string;
  readonly island: Readonly<{ component: string; documentKey: string; slot: string }>;
}
export interface PutUploadChunkRequest extends UploadRequestBase {
  readonly operation: "put_chunk";
  readonly bytes: ArrayBuffer;
  readonly checksum: string;
  readonly chunkIndex: number;
  readonly expectedRevision: string;
  readonly grant: string;
  readonly handle: UploadHandle;
  readonly idempotencyKey: string;
}
export interface CompleteUploadRequest extends UploadRequestBase {
  readonly operation: "complete";
  readonly expectedRevision: string;
  readonly grant: string;
  readonly handle: UploadHandle;
  readonly idempotencyKey: string;
  readonly wholeChecksum: string;
}
export interface CancelUploadRequest extends UploadRequestBase {
  readonly operation: "cancel";
  readonly expectedRevision: string;
  readonly grant: string;
  readonly handle: UploadHandle;
  readonly idempotencyKey: string;
}
export interface StatusUploadRequest extends UploadRequestBase {
  readonly operation: "status";
  readonly grant: string;
  readonly handle: UploadHandle;
}
export type UploadTransportRequest =
  | CreateUploadRequest
  | PutUploadChunkRequest
  | CompleteUploadRequest
  | CancelUploadRequest
  | StatusUploadRequest;
export interface UploadTransportResponse {
  readonly grant?: string;
  readonly handle?: UploadHandle;
  readonly nextChunkIndex?: number;
  readonly revision: string;
  readonly state: UploadPresentationState;
}
export interface UploadTransport {
  send(request: UploadTransportRequest): Promise<UploadTransportResponse>;
}
export interface UploadConnectivity { online(): boolean; }
export interface UploadRandomness { idempotencyKey(): string; }
export interface UploadManagerResourceSnapshot {
  readonly activeLeases: number;
  readonly bindings: number;
  readonly cleanupObligations: number;
  readonly entries: number;
  readonly generationFields: number;
  readonly observers: number;
  readonly ownedResources: number;
  readonly pendingChunkBuffers: number;
  readonly pendingChunkBytes: number;
  readonly queuedBytes: number;
  readonly queuedItems: number;
  readonly retainedStringCodeUnits: number;
  readonly waitingPermits: number;
}
export interface UploadResourceObserver {
  progressApplicationCompleted(): void;
  progressApplicationStarted(): void;
  resources(snapshot: UploadManagerResourceSnapshot): void;
}
export interface UploadFeatureOptions {
  readonly application?: UploadApplicationPort;
  readonly chunkBytes?: number;
  readonly connectivity?: UploadConnectivity;
  readonly maxActive?: number;
  readonly maxItems?: number;
  readonly maxQueueBytes?: number;
  readonly randomness?: UploadRandomness;
  readonly resourceObserver?: UploadResourceObserver;
  readonly transport?: UploadTransport;
}
export interface ReacquiredUpload {
  readonly fileIdentity: UploadFileIdentity;
  readonly grant: string;
  readonly nextChunkIndex: number;
  readonly revision: string;
  readonly state: "queued" | "transferring" | "verifying";
  readonly uploadedBytes: number;
}
export interface ReacquiredTransfer extends ReacquiredUpload {
  readonly file: File;
  readonly handle: UploadHandle;
}
export interface UploadApplicationPort {
  reacquire(request: Readonly<{
    field: string;
    fileIdentity: UploadFileIdentity;
    handle: UploadHandle;
  }>): Promise<ReacquiredUpload>;
}
export class FetchUploadTransport implements UploadTransport {
  constructor(fetchPort: typeof globalThis.fetch);
  send(request: UploadTransportRequest): Promise<UploadTransportResponse>;
}
export interface UploadResumeRequest {
  readonly field: string;
  readonly file: File;
  readonly handle: UploadHandle;
  readonly input: HTMLInputElement;
  readonly island: Element;
}
export function configureUploads(options: UploadFeatureOptions): void;
export function reacquireUpload(
  application: UploadApplicationPort | undefined,
  request: Readonly<{ field: string; file: File; handle: UploadHandle }>,
): Promise<ReacquiredTransfer>;
export function resumeUpload(request: UploadResumeRequest): Promise<void>;
export const uploadsFeature: RuntimeFeature;
export const uploadsRegistration: RuntimeFeatureRegistrationOutcome;
export default uploadsFeature;
}

declare module "@suprnova/live/async" {
import type { JsonValue, RuntimeFeature, RuntimeFeatureRegistrationOutcome } from "@suprnova/live";
export interface AsyncClock { now(): number; }
export interface AsyncRandomness { number(): number; }
export interface AsyncTimerPort {
  clearTimeout(handle: number): void;
  timeout(callback: () => void, milliseconds: number): number;
}
export interface StreamPosition { readonly epoch: bigint; readonly sequence: bigint; }
export interface PollFallbackPolicy {
  readonly intervalMs: number;
  readonly jitterRatio: number;
  readonly initial: "wait" | "immediate";
  readonly visibility: "visible" | "always";
}
export interface PollPolicy extends PollFallbackPolicy {
  readonly mode: "poll_only" | "push_only" | "hybrid";
}
export interface PollEnvironment {
  isOnline(): boolean;
  isVisible(): boolean;
  subscribe(listener: () => void): () => void;
}
export type PollStatus =
  | "current"
  | "degraded"
  | "polling"
  | "offline"
  | "suspended"
  | "closed";
export interface AsyncFreshnessObservation {
  readonly component: string;
  readonly documentKey: string;
  readonly slot: string;
  readonly state: PollStatus;
}
export type AsyncFreshnessObserver = (observation: AsyncFreshnessObservation) => void;
export interface AsyncRegisteredEventContract {
  readonly cycle:
    | Readonly<{ kind: "forbid_repeated_island" }>
    | Readonly<{ kind: "maximum_hops"; maximumHops: number }>;
  readonly maximumFanout: number;
  readonly name: string;
  readonly order: "per_source_sequence";
  readonly payloadContract: string;
  readonly schema: "json" | "null" | "boolean" | "i64" | "u64" | "f64" | "string";
  readonly source: "stream";
  readonly targets: readonly string[];
  readonly version: number;
}
export interface AuthorizedLogicalSubscription {
  readonly authorization:
    | Readonly<{ kind: "session_cookie" }>
    | Readonly<{ credential: string; kind: "bearer" }>;
  readonly baseline: StreamPosition;
  readonly descriptorBinding: string;
  readonly document: Readonly<{
    authorizationScope: string;
    origin: string;
    transport: "sse" | "websocket";
  }>;
  readonly events: readonly AsyncRegisteredEventContract[];
  readonly expiresAt: number;
  readonly fallbackPoll: PollFallbackPolicy;
  readonly heartbeatTimeoutMs: number;
  readonly presentationSignals: readonly Readonly<{
    name: string;
    schema: "null" | "boolean" | "i64" | "u64" | "string";
    scope: string;
  }>[];
  readonly reconnect: Readonly<{
    kind: "refresh_on_reconnect" | "resume_or_refresh";
    maximumAttempts: number;
    maximumDelayMs: number;
    minimumDelayMs: number;
  }>;
  readonly stream: string;
  readonly subscriptionId: string;
}
export type AsyncTransportAuthorization = AuthorizedLogicalSubscription["authorization"];
export type DocumentTransportKey = AuthorizedLogicalSubscription["document"];
export type DocumentTransportFailure =
  | "authorization_lost"
  | "heartbeat_lost"
  | "protocol_invalid"
  | "transport_lost";
export interface DocumentTransportConnectRequest {
  readonly authorization: AsyncTransportAuthorization;
  readonly key: DocumentTransportKey;
  readonly transportGeneration: number;
  failed(reason: DocumentTransportFailure): void;
  message(encoded: string): void;
  opened(): void;
}
export interface DocumentMembershipAcknowledgment {
  readonly descriptorBinding: string;
  readonly kind: "authenticated";
  readonly stream: string;
  readonly subscriptionId: string;
  readonly transportGeneration: number;
}
export interface DocumentMembershipRejection {
  readonly kind: "rejected";
  readonly reason: "authorization_lost" | "capacity" | "closed" | "timeout";
}
export type DocumentMembershipOutcome = DocumentMembershipAcknowledgment | DocumentMembershipRejection;
export interface AsyncAuthorizationRequest {
  readonly identity: Readonly<{ component: string; documentKey: string; slot: string }>;
  readonly position: StreamPosition | null;
  readonly prior: AuthorizedLogicalSubscription | null;
  readonly signal: AbortSignal;
  readonly stream: string;
}
export interface AsyncAuthorizationResult {
  readonly replay: readonly string[];
  readonly subscription: AuthorizedLogicalSubscription;
}
export interface AsyncAuthorityPort {
  authorize(request: AsyncAuthorizationRequest):
    | AsyncAuthorizationResult
    | AuthorizedLogicalSubscription
    | Promise<AsyncAuthorizationResult | AuthorizedLogicalSubscription>;
}
export interface DocumentTransportPort {
  subscribe(subscription: AuthorizedLogicalSubscription): DocumentMembershipOutcome | Promise<DocumentMembershipOutcome>;
  unsubscribe(subscriptionId: string): void;
  close(reason: "page_suspended" | "document_retired" | "transport_replaced" | "subscription_empty"): void;
}
const sseConnectionBrand: unique symbol;
export interface SseConnectionHandle {
  readonly [sseConnectionBrand]: true;
}
export interface SseMembershipControlRequest {
  readonly connection: SseConnectionHandle;
  readonly controlNonce: string;
  readonly key: DocumentTransportKey;
  readonly operation: "subscribe" | "unsubscribe";
  readonly signal: AbortSignal;
  readonly subscription: AuthorizedLogicalSubscription;
  readonly transportGeneration: number;
}
export interface SseMembershipAcknowledgment extends DocumentMembershipAcknowledgment {
  readonly connection: SseConnectionHandle;
  readonly controlNonce: string;
  readonly operation: "subscribe" | "unsubscribe";
}
export type SseMembershipOutcome = SseMembershipAcknowledgment | DocumentMembershipRejection;
export interface AsyncTransportPorts {
  eventSource(connect: DocumentTransportConnectRequest): DocumentTransportPort;
  webSocket(connect: DocumentTransportConnectRequest): DocumentTransportPort;
}
export class OriginHandshakeScheduler {
  constructor(maximum?: number);
  active(origin: string): number;
}
export interface AsyncFeatureOptions {
  readonly authority?: AsyncAuthorityPort;
  readonly clock: AsyncClock;
  readonly handshakeScheduler?: OriginHandshakeScheduler;
  readonly observeFreshness?: AsyncFreshnessObserver;
  readonly pollEnvironment?: PollEnvironment;
  readonly randomness: AsyncRandomness;
  readonly timers: AsyncTimerPort;
  readonly transports?: AsyncTransportPorts;
}
export interface BrowserAsyncTransportOptions {
  readonly eventSource: (url: string, init: Readonly<{ withCredentials: true }>) => { close(): void };
  readonly fetch: typeof globalThis.fetch;
  readonly membershipTimeoutMs: number;
  readonly sseMembership: (
    request: SseMembershipControlRequest,
  ) => SseMembershipOutcome | Promise<SseMembershipOutcome>;
  readonly timers: AsyncTimerPort;
  readonly webSocket: (url: string) => { close(code?: number, reason?: string): void; send(data: string): void };
}
export class BrowserAsyncTransportPorts implements AsyncTransportPorts {
  constructor(options: BrowserAsyncTransportOptions);
  eventSource(connect: DocumentTransportConnectRequest): DocumentTransportPort;
  webSocket(connect: DocumentTransportConnectRequest): DocumentTransportPort;
}
export function configureAsync(options: AsyncFeatureOptions): void;
export function createAsyncFeature(options: AsyncFeatureOptions): RuntimeFeature;
export const asyncFeature: RuntimeFeature;
export const asyncRegistration: RuntimeFeatureRegistrationOutcome;
export default asyncFeature;
}
