import { createHash } from "node:crypto";
import { lstat, mkdir, readdir, rm, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { build } from "esbuild";
import { minify } from "terser";

const browserRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const DEFAULT_OUTDIR = join(browserRoot, "dist");
const ENGINE_VERSION = "0.1.0";
const RUNTIME_CONTRACT_VERSION = 1;
const PROTOCOL_VERSIONS = [1, 2];
const SNAPSHOT_VERSIONS = [1];
const IDIOMORPH_VERSION = "0.7.4";
const BUILD_TIMESTAMP = "1970-01-01T00:00:00.000Z";
const CONTENT_TYPE = "text/javascript; charset=utf-8";
const CACHE_CONTROL = "public, max-age=31536000, immutable";
const COMPATIBLE_CORE = ">=0.1.0 <0.2.0";
const OUTPUTS = Object.freeze([
  {
    capability: "core@1",
    entryPoint: "src/entry-classic.ts",
    file: "suprnova-live.classic.js",
    format: "iife",
    role: "core-classic",
  },
  {
    capability: "core@1",
    entryPoint: "src/entry-esm.ts",
    file: "suprnova-live.esm.js",
    format: "esm",
    role: "core-esm",
  },
  {
    capability: "stimulus@1",
    entryPoint: "src/entry-stimulus-classic.ts",
    file: "suprnova-live.stimulus.classic.js",
    format: "iife",
    role: "stimulus-classic",
  },
  {
    capability: "stimulus@1",
    entryPoint: "src/entry-stimulus-esm.ts",
    file: "suprnova-live.stimulus.esm.js",
    format: "esm",
    role: "stimulus-esm",
  },
  {
    capability: "uploads@1",
    entryPoint: "src/entry-uploads-classic.ts",
    file: "suprnova-live.uploads.classic.js",
    format: "iife",
    role: "uploads-classic",
  },
  {
    capability: "uploads@1",
    entryPoint: "src/entry-uploads-esm.ts",
    file: "suprnova-live.uploads.esm.js",
    format: "esm",
    role: "uploads-esm",
  },
  {
    capability: "async@1",
    entryPoint: "src/entry-async-classic.ts",
    file: "suprnova-live.async.classic.js",
    format: "iife",
    role: "async-classic",
  },
  {
    capability: "async@1",
    entryPoint: "src/entry-async-esm.ts",
    file: "suprnova-live.async.esm.js",
    format: "esm",
    role: "async-esm",
  },
]);
const OUTPUT_NAMES = Object.freeze([
  "index.d.ts",
  "suprnova-live.assets.json",
  ...OUTPUTS.map(({ file }) => file),
]);
const CLEANABLE_NAMES = Object.freeze([
  ...OUTPUT_NAMES,
  ...OUTPUTS.map(({ file }) => `${file}.map`),
  "index.d.ts.map",
]);
const CLEANABLE_NAME_SET = new Set(CLEANABLE_NAMES);
const TARGETS = ["chrome111", "edge111", "firefox128", "safari16.4"];
const CORE_BANNER = `/*! Suprnova Live ${ENGINE_VERSION} | Idiomorph ${IDIOMORPH_VERSION} (0BSD) */`;
const OPTIONAL_BANNER = `/*! Suprnova Live ${ENGINE_VERSION} */`;
// Closed implementation methods only. Public API, host-port, DOM, Stimulus, and wire properties
// are deliberately absent so property mangling cannot change an integration boundary.
const INTERNAL_PROPERTY =
  /^(?:afterMorph|applicationCurrent|applicationDisposition|applyFinalState|attachConnectionObserver|attachScheduleObserver|beforeMorph|beforeUnload|beginApplication|beginFetch|beginRead|bumpEntry|cancelAll|claimRecovery|clearInFlight|commitMetadata|completeApplication|configure|connectionEpoch|consumeControlledMove|directives|dispatchEvents|disposeOwner|disposeScope|editSequence|freshRenderOperation|inFlightIntent|interruption|markInFlight|modelState|mutations|onDispose|onFinish|ownerForNode|postCommitFailure|prepareAction|presentationEmpty|promotionNonce|queueChildren|reconcile|reflectUrl|requestFreshIsland|resetRecovery|resolveNamed|restoreFocus|retireSubtree|rollbackCommit|runAll|runEffects|scanInsertion|schedulePublicCall|setFromCall|setRecovery|setTransportFeedback|setValidation|settleFeedback|settleTransport|subscribe|subscribeFeedback|takeResponse|trackIntent|unregister|userAbort|validateNoRender)$/;

const DECLARATIONS = `declare module "@suprnova/live" {
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
type FreshRenderReason = "poll" | "stream";
type FreshRenderDisposition = "queued" | "coalesced" | "retired";
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
export interface RuntimeFeatureIslandPort {
  readonly element: Element;
  readonly identity: IslandExtensionIdentity;
  enqueueFreshRender(reason: FreshRenderReason): FreshRenderDisposition;
  onDispose(dispose: () => void): void;
  queryDirectiveOwnership(
    parser: RuntimeFeatureDirectiveParser,
  ): readonly RuntimeFeatureDirectiveOwnership[];
  writePresentationSignal(element: Element, name: string, value: JsonValue): JsonValue;
}
export interface FeatureIslandController {
  dispose(): void;
  resume?(): void;
  suspend?(): void;
}
export interface FeatureDocumentController {
  connectIsland(port: RuntimeFeatureIslandPort): FeatureIslandController | undefined;
  dispose(): void;
  resume?(): void;
  suspend?(): void;
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
  readonly sri: \`sha256-\${string}\`;
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
import type { RuntimeFeature, RuntimeFeatureRegistrationOutcome } from "@suprnova/live";
export const uploadsFeature: RuntimeFeature;
export const uploadsRegistration: RuntimeFeatureRegistrationOutcome;
export default uploadsFeature;
}

declare module "@suprnova/live/async" {
import type { RuntimeFeature, RuntimeFeatureRegistrationOutcome } from "@suprnova/live";
export const asyncFeature: RuntimeFeature;
export const asyncRegistration: RuntimeFeatureRegistrationOutcome;
export default asyncFeature;
}
`;

function outputArgument(argv) {
  if (argv.length === 0) return DEFAULT_OUTDIR;
  if (argv.length === 2 && argv[0] === "--outdir" && typeof argv[1] === "string") {
    return resolve(argv[1]);
  }
  throw new Error("usage: node scripts/build.mjs [--outdir PATH]");
}

function assetRecord(output, content) {
  const digest = createHash("sha256").update(content).digest();
  return {
    file: output.file,
    role: output.role,
    bytes: content.byteLength,
    sha256: digest.toString("hex"),
    sri: `sha256-${digest.toString("base64")}`,
    capability: output.capability,
    capability_version: 1,
    compatible_core: COMPATIBLE_CORE,
    content_type: CONTENT_TYPE,
    script_kind: output.format === "esm" ? "module" : "classic",
    preload_rel: output.format === "esm" ? "modulepreload" : "preload",
    cache_control: CACHE_CONTROL,
  };
}

function normalizedInputs(metafile) {
  return Object.keys(metafile.inputs).map((name) => name.replaceAll("\\", "/"));
}

function containsInput(inputs, suffix) {
  return inputs.some((name) => name.endsWith(suffix));
}

function verifyBundleInputs(output, metafile) {
  const inputs = normalizedInputs(metafile);
  const idiomorph = containsInput(inputs, "node_modules/idiomorph/dist/idiomorph.esm.js");
  const hotwired = inputs.some((name) => name.includes("node_modules/@hotwired/stimulus/"));
  const bridge = containsInput(inputs, "src/stimulus/bridge.ts");
  const lifecycle = containsInput(inputs, "src/stimulus/lifecycle.ts");
  const optional = !output.role.startsWith("core-");
  if ((optional && idiomorph) || (!optional && !idiomorph)) {
    throw new Error(
      optional ? `optional_idiomorph_forbidden:${output.role}` : "idiomorph_not_bundled",
    );
  }
  if (hotwired) throw new Error(`stimulus_must_not_be_bundled:${output.role}`);
  if (!output.role.startsWith("stimulus-") && (bridge || lifecycle)) {
    throw new Error(`stimulus_lifecycle_forbidden:${output.role}`);
  }
  if (output.role.startsWith("stimulus-") && (!bridge || !lifecycle)) {
    throw new Error(`stimulus_lifecycle_missing:${output.role}`);
  }
  if (
    optional &&
    inputs.some(
      (name) =>
        name.endsWith("src/bootstrap.ts") ||
        name.endsWith("src/runtime/runtime.ts") ||
        name.endsWith("src/islands/discovery.ts") ||
        name.includes("src/morph/"),
    )
  ) {
    throw new Error(`optional_core_runtime_forbidden:${output.role}`);
  }
}

async function bundle(definition, outfile) {
  const result = await build({
    absWorkingDir: browserRoot,
    bundle: true,
    charset: "utf8",
    entryPoints: [definition.entryPoint],
    format: definition.format,
    legalComments: "none",
    metafile: true,
    minify: true,
    outfile,
    platform: "browser",
    sourcemap: false,
    target: TARGETS,
    treeShaking: true,
    write: false,
  });
  verifyBundleInputs(definition, result.metafile);
  const outputFile = result.outputFiles.find((file) => file.path === outfile);
  if (outputFile === undefined) throw new Error("bundle_output_missing");
  const compressed = await minify(outputFile.text, {
    compress: {
      hoist_funs: true,
      passes: 4,
    },
    ecma: 2020,
    mangle: { properties: { regex: INTERNAL_PROPERTY }, toplevel: true },
    module: definition.format === "esm",
    format: {
      comments: false,
      preamble: definition.role.startsWith("core-") ? CORE_BANNER : OPTIONAL_BANNER,
    },
  });
  if (compressed.code === undefined) throw new Error("terser_output_missing");
  return Buffer.from(`${compressed.code}\n`, "utf8");
}

export async function buildRuntimeAssets(outdir = DEFAULT_OUTDIR) {
  const destination = resolve(outdir);
  try {
    await mkdir(destination, { recursive: true });
  } catch {
    throw new Error("build_output_directory_dirty");
  }
  // This local build gate covers pre-existing destination state. Deployment tooling owns atomic
  // publication and protection against concurrent hostile path replacement.
  let destinationMetadata;
  try {
    destinationMetadata = await lstat(destination);
  } catch {
    throw new Error("build_output_directory_dirty");
  }
  if (destinationMetadata.isSymbolicLink() || !destinationMetadata.isDirectory()) {
    throw new Error("build_output_directory_dirty");
  }
  const existing = await readdir(destination, { withFileTypes: true });
  if (existing.some((entry) => !entry.isFile() || !CLEANABLE_NAME_SET.has(entry.name))) {
    throw new Error("build_output_directory_dirty");
  }
  for (const name of CLEANABLE_NAMES) await rm(join(destination, name), { force: true });

  const assets = [];
  for (const output of OUTPUTS) {
    const content = await bundle(output, join(destination, output.file));
    await writeFile(join(destination, output.file), content);
    assets.push(assetRecord(output, content));
  }
  await writeFile(join(destination, "index.d.ts"), DECLARATIONS, "utf8");

  const manifest = {
    schema_version: 2,
    engine_version: ENGINE_VERSION,
    runtime_contract_version: RUNTIME_CONTRACT_VERSION,
    protocol_versions: PROTOCOL_VERSIONS,
    snapshot_versions: SNAPSHOT_VERSIONS,
    built_at: BUILD_TIMESTAMP,
    assets,
    provenance: {
      idiomorph: { name: "idiomorph", version: IDIOMORPH_VERSION, license: "0BSD", bundled: true },
    },
  };
  await writeFile(
    join(destination, "suprnova-live.assets.json"),
    `${JSON.stringify(manifest, null, 2)}\n`,
    "utf8",
  );
  return Object.freeze({ destination, files: OUTPUT_NAMES });
}

const invokedPath = process.argv[1] === undefined ? "" : resolve(process.argv[1]);
if (invokedPath === fileURLToPath(import.meta.url)) {
  await buildRuntimeAssets(outputArgument(process.argv.slice(2)));
}
