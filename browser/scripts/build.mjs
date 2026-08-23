import { createHash } from "node:crypto";
import { mkdir, rm, writeFile } from "node:fs/promises";
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
const OUTPUT_NAMES = [
  "index.d.ts",
  "suprnova-live.assets.json",
  "suprnova-live.classic.js",
  "suprnova-live.esm.js",
];
const TARGETS = ["chrome111", "edge111", "firefox128", "safari16.4"];
const BANNER = `/*! Suprnova Live ${ENGINE_VERSION} | Idiomorph ${IDIOMORPH_VERSION} (0BSD) */`;
// Closed implementation methods only. Public API, host-port, DOM, Stimulus, and wire properties
// are deliberately absent so property mangling cannot change an integration boundary.
const INTERNAL_PROPERTY =
  /^(?:afterMorph|applicationCurrent|applicationDisposition|applyFinalState|attachConnectionObserver|attachScheduleObserver|beforeMorph|beforeUnload|beginApplication|beginFetch|beginRead|bumpEntry|cancelAll|claimRecovery|clearInFlight|commitMetadata|completeApplication|configure|connectionEpoch|consumeControlledMove|directives|dispatchEvents|disposeOwner|disposeScope|editSequence|freshRenderOperation|inFlightIntent|interruption|markInFlight|modelState|mutations|onDispose|onFinish|ownerForNode|postCommitFailure|prepareAction|presentationEmpty|promotionNonce|queueChildren|reconcile|reflectUrl|requestFreshIsland|resetRecovery|resolveNamed|restoreFocus|retireSubtree|rollbackCommit|runAll|runEffects|scanInsertion|schedulePublicCall|setFromCall|setRecovery|setTransportFeedback|setValidation|settleFeedback|settleTransport|subscribe|subscribeFeedback|takeResponse|trackIntent|unregister|userAbort|validateNoRender)$/;

const DECLARATIONS = `export type DiagnosticMode = "off" | "errors" | "verbose";
export type RuntimeStatus = "running" | "suspended" | "stopped";
export type JsonValue = null | boolean | number | string | JsonArray | JsonObject;
export interface JsonArray extends ReadonlyArray<JsonValue> {
  readonly [index: number]: JsonValue;
}
export interface JsonObject {
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
export interface IslandExtensionIdentity {
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
export interface EffectInvocation {
  readonly name: string;
  readonly version?: number;
  readonly payload: unknown;
}
export type EffectRunStatus =
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
  readonly bytes: number;
  readonly sha256: string;
  readonly sri: \`sha256-\${string}\`;
  readonly content_type: "text/javascript; charset=utf-8";
  readonly script_kind: "module" | "classic";
  readonly preload_rel: "modulepreload" | "preload";
  readonly cache_control: "public, max-age=31536000, immutable";
}
export interface RuntimeAssetManifest {
  readonly schema_version: 1;
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
export declare const version: "0.1.0";
export declare const runtimeContractVersion: 1;
export declare const supportedProtocolVersions: readonly [1, 2];
export declare const RUNTIME_SYMBOL: symbol;
export declare function boot(options?: BootstrapOptions): RuntimeHandle;
declare const api: SuprnovaLivePublicApi;
export default api;
`;

function outputArgument(argv) {
  if (argv.length === 0) return DEFAULT_OUTDIR;
  if (argv.length === 2 && argv[0] === "--outdir" && typeof argv[1] === "string") {
    return resolve(argv[1]);
  }
  throw new Error("usage: node scripts/build.mjs [--outdir PATH]");
}

function assetRecord(file, content, scriptKind) {
  const digest = createHash("sha256").update(content).digest();
  return {
    file,
    bytes: content.byteLength,
    sha256: digest.toString("hex"),
    sri: `sha256-${digest.toString("base64")}`,
    content_type: CONTENT_TYPE,
    script_kind: scriptKind,
    preload_rel: scriptKind === "module" ? "modulepreload" : "preload",
    cache_control: CACHE_CONTROL,
  };
}

async function bundle(entryPoint, format, outfile) {
  const result = await build({
    absWorkingDir: browserRoot,
    bundle: true,
    charset: "utf8",
    entryPoints: [entryPoint],
    format,
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
  const idiomorphInput = Object.keys(result.metafile.inputs).some((name) =>
    name.replaceAll("\\", "/").endsWith("node_modules/idiomorph/dist/idiomorph.esm.js"),
  );
  if (!idiomorphInput) throw new Error("idiomorph_not_bundled");
  const stimulusInput = Object.keys(result.metafile.inputs).some((name) =>
    name.replaceAll("\\", "/").includes("node_modules/@hotwired/stimulus/"),
  );
  if (stimulusInput) throw new Error("stimulus_must_not_be_bundled");
  const output = result.outputFiles.find((file) => file.path === outfile);
  if (output === undefined) throw new Error("bundle_output_missing");
  const compressed = await minify(output.text, {
    compress: {
      hoist_funs: true,
      passes: 4,
    },
    ecma: 2020,
    mangle: { properties: { regex: INTERNAL_PROPERTY }, toplevel: true },
    module: format === "esm",
    format: { comments: false, preamble: BANNER },
  });
  if (compressed.code === undefined) throw new Error("terser_output_missing");
  return Buffer.from(`${compressed.code}\n`, "utf8");
}

export async function buildRuntimeAssets(outdir = DEFAULT_OUTDIR) {
  const destination = resolve(outdir);
  await mkdir(destination, { recursive: true });
  for (const name of OUTPUT_NAMES) await rm(join(destination, name), { force: true });
  for (const name of [
    "suprnova-live.esm.js.map",
    "suprnova-live.classic.js.map",
    "index.d.ts.map",
  ]) {
    await rm(join(destination, name), { force: true });
  }

  const classicName = "suprnova-live.classic.js";
  const esmName = "suprnova-live.esm.js";
  const classic = await bundle("src/entry-classic.ts", "iife", join(destination, classicName));
  const esm = await bundle("src/entry-esm.ts", "esm", join(destination, esmName));
  await writeFile(join(destination, classicName), classic);
  await writeFile(join(destination, esmName), esm);
  await writeFile(join(destination, "index.d.ts"), DECLARATIONS, "utf8");

  const manifest = {
    schema_version: 1,
    engine_version: ENGINE_VERSION,
    runtime_contract_version: RUNTIME_CONTRACT_VERSION,
    protocol_versions: PROTOCOL_VERSIONS,
    snapshot_versions: SNAPSHOT_VERSIONS,
    built_at: BUILD_TIMESTAMP,
    assets: [assetRecord(classicName, classic, "classic"), assetRecord(esmName, esm, "module")],
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
