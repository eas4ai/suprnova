import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import vm from "node:vm";

import { build } from "esbuild";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { PRODUCTION_BUILD_HOOK_TIMEOUT_MS } from "./support/production-build.js";

const OUTPUT_NAMES = Object.freeze([
  "index.d.ts",
  "suprnova-live.assets.json",
  "suprnova-live.async.classic.js",
  "suprnova-live.async.esm.js",
  "suprnova-live.classic.js",
  "suprnova-live.esm.js",
  "suprnova-live.stimulus.classic.js",
  "suprnova-live.stimulus.esm.js",
  "suprnova-live.uploads.classic.js",
  "suprnova-live.uploads.esm.js",
] as const);

const ROLES = Object.freeze([
  "async-classic",
  "async-esm",
  "core-classic",
  "core-esm",
  "stimulus-classic",
  "stimulus-esm",
  "uploads-classic",
  "uploads-esm",
] as const);

type AssetRole = (typeof ROLES)[number];

interface OptionalAsset {
  readonly file: string;
  readonly role: AssetRole;
  readonly bytes: number;
  readonly sha256: string;
  readonly sri: string;
  readonly capability: "async@1" | "core@1" | "stimulus@1" | "uploads@1";
  readonly capability_version: 1;
  readonly compatible_core: ">=0.1.0 <0.2.0";
  readonly script_kind: "module" | "classic";
}

interface OptionalManifest {
  readonly schema_version: 2;
  readonly protocol_versions: readonly [1, 2];
  readonly assets: readonly OptionalAsset[];
}

const browserRoot = new URL("../", import.meta.url);
const buildScript = new URL("scripts/build.mjs", browserRoot);
const typeScript = new URL("node_modules/typescript/bin/tsc", browserRoot);
let temporaryRoot = "";
let outputDirectory = "";

function runBuild(outdir: string): void {
  execFileSync(process.execPath, [buildScript.pathname, "--outdir", outdir], {
    cwd: new URL("..", browserRoot),
    stdio: "pipe",
  });
}

async function inputs(entryPoint: string): Promise<readonly string[]> {
  const result = await build({
    absWorkingDir: browserRoot.pathname,
    bundle: true,
    entryPoints: [entryPoint],
    format: entryPoint.endsWith("-classic.ts") ? "iife" : "esm",
    metafile: true,
    platform: "browser",
    target: ["chrome111", "edge111", "firefox128", "safari16.4"],
    treeShaking: true,
    write: false,
  });
  return Object.freeze(
    Object.keys(result.metafile.inputs).map((name) => name.replace(/\\/gu, "/")),
  );
}

beforeAll(async () => {
  temporaryRoot = await mkdtemp(join(tmpdir(), "suprnova-live-optional-assets-"));
  outputDirectory = join(temporaryRoot, "first");
  runBuild(outputDirectory);
}, PRODUCTION_BUILD_HOOK_TIMEOUT_MS);

afterAll(async () => {
  if (temporaryRoot.length > 0) await rm(temporaryRoot, { recursive: true, force: true });
});

describe("role-typed optional production artifacts", () => {
  it("emits exactly eight scripts, one declaration, and one schema-2 manifest", async () => {
    expect((await readdir(outputDirectory)).sort()).toEqual(OUTPUT_NAMES);
    const manifest = JSON.parse(
      await readFile(join(outputDirectory, "suprnova-live.assets.json"), "utf8"),
    ) as OptionalManifest;
    expect(manifest.schema_version).toBe(2);
    expect(manifest.protocol_versions).toEqual([1, 2]);
    expect(manifest.assets.map(({ role }) => role).sort()).toEqual(ROLES);
  });

  it("binds every role to independent bytes, hash, SRI, capability, and compatibility", async () => {
    const manifest = JSON.parse(
      await readFile(join(outputDirectory, "suprnova-live.assets.json"), "utf8"),
    ) as OptionalManifest;
    const capabilities: Readonly<Record<AssetRole, OptionalAsset["capability"]>> = {
      "async-classic": "async@1",
      "async-esm": "async@1",
      "core-classic": "core@1",
      "core-esm": "core@1",
      "stimulus-classic": "stimulus@1",
      "stimulus-esm": "stimulus@1",
      "uploads-classic": "uploads@1",
      "uploads-esm": "uploads@1",
    };
    expect(new Set(manifest.assets.map(({ sha256 }) => sha256))).toHaveLength(8);
    for (const asset of manifest.assets) {
      const content = await readFile(join(outputDirectory, asset.file));
      const digest = createHash("sha256").update(content).digest();
      expect(asset.bytes).toBe(content.byteLength);
      expect(asset.sha256).toBe(digest.toString("hex"));
      expect(asset.sri).toBe(`sha256-${digest.toString("base64")}`);
      expect(asset.capability).toBe(capabilities[asset.role]);
      expect(asset.capability_version).toBe(1);
      expect(asset.compatible_core).toBe(">=0.1.0 <0.2.0");
      expect(asset.script_kind).toBe(asset.role.endsWith("-esm") ? "module" : "classic");
    }
  });

  it("publishes typed optional ESM registrations in the single declaration file", async () => {
    const declarations = await readFile(join(outputDirectory, "index.d.ts"), "utf8");
    expect(declarations).toContain("export interface FeatureDocumentController");
    expect(declarations).toContain("export type RuntimeFeatureIslandPort");
    expect(declarations).toContain("export interface AsyncRuntimeIslandPort");
    expect(declarations).toContain("export interface UploadsRuntimeIslandPort");
    expect(declarations).toContain("interface ValidatedAsyncDescriptorCapability");
    expect(declarations).toContain(
      "consumeRegisteredEventCapability(\n    descriptor: ValidatedAsyncDescriptorCapability,",
    );
    expect(declarations).toContain(
      "writePresentationSignal(scope: string, name: string, value: JsonValue): JsonValue;",
    );
    expect(declarations).toContain("scope: string;");
    const asyncPort = declarations.slice(
      declarations.indexOf("export interface AsyncRuntimeIslandPort"),
      declarations.indexOf("export interface UploadsRuntimeIslandPort"),
    );
    expect(asyncPort).not.toContain("proposeUploadHandle");
    expect(asyncPort).not.toContain("authorizeRegisteredEvents");
    expect(declarations).toContain("export type FreshRenderCompletion");
    expect(declarations).toContain("export interface RuntimeConnectivity");
    expect(declarations).not.toContain("export type RuntimeFeatureName");
    expect(declarations).toContain("export const stimulusRegistration");
    expect(declarations).toContain("export const uploadsFeature: RuntimeFeature");
    expect(declarations).toContain("export function configureUploads");
    expect(declarations).toContain("export function resumeUpload");
    expect(declarations).not.toContain("export function createUploadsFeature");
    expect(declarations).toContain("export const asyncFeature: RuntimeFeature");
    expect(declarations).toContain("export function configureAsync");
    expect(declarations).toContain("export interface AsyncFreshnessObservation");
    expect(declarations).toContain("export interface ClassicFeatureSurface");
  });

  it(
    "resolves exact core and optional types through real package exports",
    async () => {
      const consumer = join(temporaryRoot, "type-consumer");
      const installed = join(consumer, "node_modules", "@suprnova", "live");
      await mkdir(join(installed, "dist"), { recursive: true });
      await copyFile(new URL("../package.json", import.meta.url), join(installed, "package.json"));
      await copyFile(join(outputDirectory, "index.d.ts"), join(installed, "dist", "index.d.ts"));
      await writeFile(join(consumer, "package.json"), '{"type":"module"}\n', "utf8");
      await writeFile(
        join(consumer, "tsconfig.json"),
        `${JSON.stringify({
          compilerOptions: {
            lib: ["ES2020", "DOM"],
            module: "NodeNext",
            moduleResolution: "NodeNext",
            noEmit: true,
            skipLibCheck: false,
            strict: true,
            target: "ES2020",
            types: [],
          },
          files: ["consumer.ts"],
        })}\n`,
        "utf8",
      );
      await writeFile(
        join(consumer, "consumer.ts"),
        `import live, {
        boot,
        RUNTIME_SYMBOL,
        runtimeContractVersion,
        supportedProtocolVersions,
        version,
        type BootstrapOptions,
        type AsyncRuntimeIslandPort,
        CLASSIC_FEATURE_SYMBOL,
        type ClassicFeatureSurface,
        type EffectContext,
        type EffectInvocation,
        type EffectRegistration,
        type EffectRunOutcome,
        type FeatureDocumentController,
        type FeatureIslandController,
        type FreshRenderCompletion,
        type FreshRenderCompletionObserver,
        type JsonValue,
        type NavigationPort,
        type PayloadSchema,
        type RuntimeAsset,
        type RuntimeAssetCapability,
        type RuntimeAssetManifest,
        type RuntimeAssetRole,
        type RuntimeCallContext,
        type RuntimeCallRegistration,
        type RuntimeClock,
        type RuntimeConnectivity,
        type RuntimeFeature,
        type RuntimeFeatureDocumentContext,
        type RuntimeFeatureIslandPort,
        type RuntimeFeatureRegistrationOutcome,
        type RuntimeFeatures,
        type RuntimeHandle,
        type RuntimeObserverFactory,
        type RuntimePortOverrides,
        type RuntimeRandomness,
        type RuntimeScheduler,
        type RuntimeStatus,
        type StimulusApplicationPort,
        type StimulusBootstrapOptions,
        type StimulusContinuity,
        type StimulusContinuityRoot,
        type StimulusMorphBridge,
        type SuprnovaLivePublicApi,
        type TransportPort,
        type UploadsRuntimeIslandPort,
      } from "@suprnova/live";
      import runtime, { boot as runtimeBoot } from "@suprnova/live/runtime";
      import stimulus, {
        installStimulusAdapter,
        stimulusRegistration,
      } from "@suprnova/live/stimulus";
      import uploads, {
        configureUploads,
        FetchUploadTransport,
        reacquireUpload,
        resumeUpload,
        uploadsFeature,
        uploadsRegistration,
        type ReacquiredTransfer,
        type UploadApplicationPort,
        type UploadFeatureOptions,
        type UploadResumeRequest,
        type UploadTransport,
        type UploadTransportRequest,
        type UploadTransportResponse,
      } from "@suprnova/live/uploads";
      import asynchronous, {
        asyncFeature,
        asyncRegistration,
        configureAsync,
        type AsyncFeatureOptions,
        type AsyncFreshnessObservation,
        type AsyncFreshnessObserver,
        type AsyncTransportPorts,
        type BrowserAsyncTransportOptions,
        type DocumentMembershipOutcome,
        type DocumentTransportConnectRequest,
        type DocumentTransportPort,
        type SseMembershipAcknowledgment,
        type SseMembershipControlRequest,
      } from "@suprnova/live/async";
      const liveApi: SuprnovaLivePublicApi = live;
      const runtimeApi: SuprnovaLivePublicApi = runtime;
      const upload: RuntimeFeature = uploads;
      const asynchronousFeature: RuntimeFeature = asynchronous;
      const asyncOptions = null as AsyncFeatureOptions | null;
      const freshnessObservation = null as AsyncFreshnessObservation | null;
      const freshnessObserver = null as AsyncFreshnessObserver | null;
      const freshRenderCompletion = null as FreshRenderCompletion | null;
      const freshRenderCompletionObserver = null as FreshRenderCompletionObserver | null;
      const customTransport: AsyncTransportPorts = {
        eventSource(connect: DocumentTransportConnectRequest): DocumentTransportPort {
          return {
            close() {},
            subscribe(subscription): DocumentMembershipOutcome {
              return {
                descriptorBinding: subscription.descriptorBinding,
                kind: "authenticated",
                stream: subscription.stream,
                subscriptionId: subscription.subscriptionId,
                transportGeneration: connect.transportGeneration,
              };
            },
            unsubscribe() {},
          };
        },
        webSocket(connect: DocumentTransportConnectRequest): DocumentTransportPort {
          return this.eventSource(connect);
        },
      };
      const sseMembership: BrowserAsyncTransportOptions["sseMembership"] = (
        request: SseMembershipControlRequest,
      ): SseMembershipAcknowledgment => ({
        connection: request.connection,
        controlNonce: request.controlNonce,
        descriptorBinding: request.subscription.descriptorBinding,
        kind: "authenticated",
        operation: request.operation,
        stream: request.subscription.stream,
        subscriptionId: request.subscription.subscriptionId,
        transportGeneration: request.transportGeneration,
      });
      const classicSurface = null as ClassicFeatureSurface | null;
      const stimulusOutcome: RuntimeFeatureRegistrationOutcome = stimulus;
      const uploadApplication = null as UploadApplicationPort | null;
      const uploadOptions = null as UploadFeatureOptions | null;
      const uploadRequest = null as UploadResumeRequest | null;
      const uploadTransport = null as UploadTransport | null;
      const uploadTransportRequest = null as UploadTransportRequest | null;
      const uploadTransportResponse = null as UploadTransportResponse | null;
      const reacquiredTransfer = null as ReacquiredTransfer | null;
      const portOverrides: RuntimePortOverrides = {
        connectivity: { isOnline: () => true },
      };
      type RootTypeExports = [
        AsyncRuntimeIslandPort,
        BootstrapOptions,
        ClassicFeatureSurface,
        EffectContext,
        EffectInvocation,
        EffectRegistration,
        EffectRunOutcome,
        FeatureDocumentController,
        FeatureIslandController,
        FreshRenderCompletion,
        FreshRenderCompletionObserver,
        JsonValue,
        NavigationPort,
        PayloadSchema,
        RuntimeAsset,
        RuntimeAssetCapability,
        RuntimeAssetManifest,
        RuntimeAssetRole,
        RuntimeCallContext,
        RuntimeCallRegistration,
        RuntimeClock,
        RuntimeConnectivity,
        RuntimeFeature,
        RuntimeFeatureDocumentContext,
        RuntimeFeatureIslandPort,
        RuntimeFeatureRegistrationOutcome,
        RuntimeFeatures,
        RuntimeHandle,
        RuntimeObserverFactory,
        RuntimePortOverrides,
        RuntimeRandomness,
        RuntimeScheduler,
        RuntimeStatus,
        StimulusApplicationPort,
        StimulusBootstrapOptions,
        StimulusContinuity,
        StimulusContinuityRoot,
        StimulusMorphBridge,
        SuprnovaLivePublicApi,
        TransportPort,
        UploadsRuntimeIslandPort,
      ];
      const rootTypeExports = null as RootTypeExports | null;
      void [
        boot,
        RUNTIME_SYMBOL,
        runtimeContractVersion,
        supportedProtocolVersions,
        version,
        runtimeBoot,
        installStimulusAdapter,
        stimulusRegistration,
        uploadsFeature,
        uploadsRegistration,
        configureUploads,
        FetchUploadTransport,
        reacquireUpload,
        resumeUpload,
        asyncFeature,
        asyncRegistration,
        configureAsync,
        CLASSIC_FEATURE_SYMBOL,
        liveApi,
        runtimeApi,
        upload,
        asynchronousFeature,
        asyncOptions,
        freshnessObservation,
        freshnessObserver,
        freshRenderCompletion,
        freshRenderCompletionObserver,
        customTransport,
        sseMembership,
        classicSurface,
        stimulusOutcome,
        uploadApplication,
        uploadOptions,
        uploadRequest,
        uploadTransport,
        uploadTransportRequest,
        uploadTransportResponse,
        reacquiredTransfer,
        portOverrides,
        rootTypeExports,
      ];
      `,
        "utf8",
      );
      const allowed = spawnSync(
        process.execPath,
        [typeScript.pathname, "--project", "tsconfig.json"],
        {
          cwd: consumer,
          encoding: "utf8",
        },
      );
      expect(`${allowed.stdout}${allowed.stderr}`).toBe("");
      expect(allowed.status).toBe(0);

      await writeFile(
        join(consumer, "consumer.ts"),
        `import type {
        DocumentTransportConnectRequest,
        DocumentTransportPort,
      } from "@suprnova/live/async";
      const voidSubscribe: DocumentTransportPort = {
        close() {},
        subscribe() {},
        unsubscribe() {},
      };
      const missingGeneration: DocumentTransportConnectRequest = {
        authorization: { kind: "session_cookie" },
        key: { authorizationScope: "scope", origin: "https://app.test", transport: "sse" },
        failed() {},
        message() {},
        opened() {},
      };
      void [voidSubscribe, missingGeneration];
      `,
        "utf8",
      );
      const invalidAsyncPort = spawnSync(
        process.execPath,
        [typeScript.pathname, "--project", "tsconfig.json", "--pretty", "false"],
        { cwd: consumer, encoding: "utf8" },
      );
      const invalidAsyncDiagnostics = `${invalidAsyncPort.stdout}${invalidAsyncPort.stderr}`;
      expect(invalidAsyncPort.status).toBe(2);
      expect(invalidAsyncDiagnostics).toContain("Type 'void' is not assignable");
      expect(invalidAsyncDiagnostics).toContain("transportGeneration");

      await writeFile(
        join(consumer, "consumer.ts"),
        `import uploadsDefault, { asyncFeature, boot } from "@suprnova/live/uploads";
      import {
        uploadsFeature,
        type DiagnosticMode,
        type EffectRunStatus,
        type IslandExtensionIdentity,
        type JsonArray,
        type JsonObject,
        type RuntimeFeatureName,
      } from "@suprnova/live";
      import type { SuprnovaLivePublicApi } from "@suprnova/live";
      const invalidCore: SuprnovaLivePublicApi = uploadsDefault;
      type Forbidden = DiagnosticMode | EffectRunStatus | IslandExtensionIdentity | JsonArray | JsonObject | RuntimeFeatureName;
      void [asyncFeature, boot, uploadsFeature, invalidCore, null as Forbidden | null];
      `,
        "utf8",
      );
      const forbidden = spawnSync(
        process.execPath,
        [typeScript.pathname, "--project", "tsconfig.json", "--pretty", "false"],
        { cwd: consumer, encoding: "utf8" },
      );
      const diagnostics = `${forbidden.stdout}${forbidden.stderr}`;
      expect(forbidden.status).toBe(2);
      expect(diagnostics).toContain("has no exported member 'asyncFeature'");
      expect(diagnostics).toContain("has no exported member 'boot'");
      expect(diagnostics).toContain("has no exported member 'uploadsFeature'");
      expect(diagnostics).toContain("has no exported member 'DiagnosticMode'");
      expect(diagnostics).toContain("has no exported member 'EffectRunStatus'");
      expect(diagnostics).toContain("has no exported member 'IslandExtensionIdentity'");
      expect(diagnostics).toContain("has no exported member 'JsonArray'");
      expect(diagnostics).toContain("has no exported member 'JsonObject'");
      expect(diagnostics).toMatch(/has no exported member (?:named )?'RuntimeFeatureName'/u);
      expect(diagnostics).toContain("is not assignable to type 'SuprnovaLivePublicApi'");

      await writeFile(
        join(consumer, "consumer.ts"),
        `import runtime, { boot, type RuntimeHandle } from "@suprnova/live/runtime";
      const handle = null as RuntimeHandle | null;
      void [runtime, boot, handle];
      `,
        "utf8",
      );
      const runtimeOnly = spawnSync(
        process.execPath,
        [typeScript.pathname, "--project", "tsconfig.json", "--pretty", "false"],
        { cwd: consumer, encoding: "utf8" },
      );
      expect(`${runtimeOnly.stdout}${runtimeOnly.stderr}`).toBe("");
      expect(runtimeOnly.status).toBe(0);
    },
    PRODUCTION_BUILD_HOOK_TIMEOUT_MS,
  );

  it("keeps optional graphs out of core implementation and third-party runtime code", async () => {
    const entries = [
      "src/entry-classic.ts",
      "src/entry-esm.ts",
      "src/entry-stimulus-classic.ts",
      "src/entry-stimulus-esm.ts",
      "src/entry-uploads-classic.ts",
      "src/entry-uploads-esm.ts",
      "src/entry-async-classic.ts",
      "src/entry-async-esm.ts",
    ];
    const graphs = new Map<string, readonly string[]>();
    for (const entry of entries) graphs.set(entry, await inputs(entry));
    for (const graph of graphs.values()) {
      expect(graph.some((name) => name.includes("node_modules/@hotwired/stimulus/"))).toBe(false);
    }
    for (const entry of entries.filter(
      (name) => !name.includes("entry-classic") && !name.includes("entry-esm"),
    )) {
      const graph = graphs.get(entry) ?? [];
      expect(
        graph.some((name) => name.endsWith("node_modules/idiomorph/dist/idiomorph.esm.js")),
      ).toBe(false);
      expect(graph.some((name) => name.endsWith("src/bootstrap.ts"))).toBe(false);
      expect(graph.some((name) => name.endsWith("src/runtime/runtime.ts"))).toBe(false);
      expect(graph.some((name) => name.endsWith("src/islands/discovery.ts"))).toBe(false);
    }
    for (const entry of entries.filter((name) => !name.includes("stimulus"))) {
      const graph = graphs.get(entry) ?? [];
      expect(graph.some((name) => name.endsWith("src/stimulus/bridge.ts"))).toBe(false);
      expect(graph.some((name) => name.endsWith("src/stimulus/lifecycle.ts"))).toBe(false);
    }
    for (const entry of entries.filter((name) => name.includes("stimulus"))) {
      const graph = graphs.get(entry) ?? [];
      expect(graph.some((name) => name.endsWith("src/stimulus/bridge.ts"))).toBe(true);
      expect(graph.some((name) => name.endsWith("src/stimulus/lifecycle.ts"))).toBe(true);
    }
  });

  it("registers ESM features and preserves classic singleton ownership across duplicate loads", async () => {
    const esmProbe = `
      const symbol = Symbol.for("suprnova.live.features.v1");
      const adopt = Symbol.for("suprnova.live.features.v1.adopt");
      const root = ${JSON.stringify(outputDirectory)};
      const stimulus = await import(new URL("suprnova-live.stimulus.esm.js", "file://" + root + "/"));
      const uploads = await import(new URL("suprnova-live.uploads.esm.js", "file://" + root + "/"));
      const asynchronous = await import(new URL("suprnova-live.async.esm.js", "file://" + root + "/"));
      const surface = Reflect.get(globalThis, symbol);
      if (surface?.version !== 1 || typeof Reflect.get(surface, adopt) !== "function") process.exit(2);
      if (stimulus.stimulusRegistration !== "registered") process.exit(3);
      if (uploads.uploadsRegistration !== "registered" || uploads.default !== uploads.uploadsFeature) process.exit(4);
      if (typeof uploads.configureUploads !== "function" || typeof uploads.resumeUpload !== "function") process.exit(7);
      if (typeof uploads.FetchUploadTransport !== "function" || typeof uploads.reacquireUpload !== "function") process.exit(8);
      if ("createUploadsFeature" in uploads) process.exit(9);
      if (asynchronous.asyncRegistration !== "registered" || asynchronous.default !== asynchronous.asyncFeature) process.exit(5);
      const core = await import(new URL("suprnova-live.esm.js", "file://" + root + "/"));
      if (typeof core.boot !== "function" || Reflect.get(globalThis, symbol) !== surface) process.exit(6);
    `;
    expect(() =>
      execFileSync(process.execPath, ["--input-type=module", "--eval", esmProbe], {
        stdio: "pipe",
      }),
    ).not.toThrow();

    const contextGlobal: Record<string, unknown> = {};
    contextGlobal["window"] = contextGlobal;
    const context = vm.createContext(contextGlobal);
    for (const name of [
      "suprnova-live.stimulus.classic.js",
      "suprnova-live.uploads.classic.js",
      "suprnova-live.async.classic.js",
    ]) {
      const source = await readFile(join(outputDirectory, name), "utf8");
      vm.runInContext(source, context);
      const first = vm.runInContext(
        'globalThis[Symbol.for("suprnova.live.features.v1")]',
        context,
      ) as unknown;
      vm.runInContext(source, context);
      const second = vm.runInContext(
        'globalThis[Symbol.for("suprnova.live.features.v1")]',
        context,
      ) as unknown;
      expect(second).toBe(first);
    }
    const beforeCore = vm.runInContext(
      'globalThis[Symbol.for("suprnova.live.features.v1")]',
      context,
    ) as unknown;
    vm.runInContext(
      await readFile(join(outputDirectory, "suprnova-live.classic.js"), "utf8"),
      context,
    );
    const afterCore = vm.runInContext(
      'globalThis[Symbol.for("suprnova.live.features.v1")]',
      context,
    ) as unknown;
    const bootType = vm.runInContext("typeof globalThis.SuprnovaLive.boot", context) as unknown;
    expect(afterCore).toBe(beforeCore);
    expect(bootType).toBe("function");
  });

  it("performs one registration attempt per fresh upload or async classic evaluation", async () => {
    const setup = `
      globalThis.registrationAttempts = 0;
      const surface = {
        configureAsync() {},
        version: 1,
        register() {
          globalThis.registrationAttempts += 1;
          return "registered";
        },
      };
      Object.defineProperty(surface, Symbol.for("suprnova.live.features.v1.adopt"), {
        value: () => undefined,
      });
      Object.defineProperty(surface, Symbol.for("suprnova.live.features.v1.stimulus-adapter"), {
        value: () => "registered",
      });
      Object.defineProperty(surface, Symbol.for("suprnova.live.features.v1.async-configuration"), {
        value: () => undefined,
      });
      Object.freeze(surface);
      Object.defineProperty(globalThis, Symbol.for("suprnova.live.features.v1"), { value: surface });
    `;
    for (const name of ["suprnova-live.uploads.classic.js", "suprnova-live.async.classic.js"]) {
      const context = vm.createContext({});
      vm.runInContext(setup, context);
      vm.runInContext(await readFile(join(outputDirectory, name), "utf8"), context);
      expect(vm.runInContext("globalThis.registrationAttempts", context) as unknown).toBe(1);
    }
  });

  it("exposes an inert-by-default but configurable classic async preboot surface", async () => {
    const source = await readFile(join(outputDirectory, "suprnova-live.async.classic.js"), "utf8");
    const unconfigured = vm.createContext({});
    vm.runInContext(source, unconfigured);
    expect(
      vm.runInContext(
        'typeof globalThis[Symbol.for("suprnova.live.features.v1")].configureAsync',
        unconfigured,
      ),
    ).toBe("function");

    const configured = vm.createContext({});
    vm.runInContext(source, configured);
    vm.runInContext(
      `globalThis[Symbol.for("suprnova.live.features.v1")].configureAsync({
        authority: { authorize() { throw new Error("not_invoked_before_boot"); } },
        clock: { now() { return 0; } },
        randomness: { number() { return 0.5; } },
        timers: { clearTimeout() {}, timeout() { return 1; } },
        transports: { eventSource() { throw new Error("not_invoked_before_boot"); }, webSocket() { throw new Error("not_invoked_before_boot"); } },
      })`,
      configured,
    );
    expect(() => {
      vm.runInContext(
        `globalThis[Symbol.for("suprnova.live.features.v1")].configureAsync({})`,
        configured,
      );
    }).toThrow("async_configuration_locked");
  });

  it("contains no source maps or runtime code-generation forms and rebuilds byte-identically", async () => {
    for (const name of OUTPUT_NAMES.filter((name) => name.endsWith(".js"))) {
      const source = await readFile(join(outputDirectory, name), "utf8");
      expect(source).not.toContain("sourceMappingURL");
      expect(source).not.toMatch(/\beval\s*\(/u);
      expect(source).not.toMatch(/\bnew\s+Function\b/u);
      expect(source).not.toMatch(/\bimport\s*\(/u);
      expect(source).not.toContain("@hotwired/stimulus");
    }
    const second = join(temporaryRoot, "second");
    runBuild(second);
    expect((await readdir(second)).sort()).toEqual(OUTPUT_NAMES);
    for (const name of OUTPUT_NAMES) {
      expect(await readFile(join(second, name))).toEqual(
        await readFile(join(outputDirectory, name)),
      );
    }
  }, 15_000);

  it("rejects unknown output files and directories without deleting them", async () => {
    const dirty = join(temporaryRoot, "dirty-output");
    await mkdir(dirty);
    const unknownFile = join(dirty, "operator-notes.txt");
    await writeFile(unknownFile, "retain me\n", "utf8");
    const fileResult = spawnSync(process.execPath, [buildScript.pathname, "--outdir", dirty], {
      encoding: "utf8",
    });
    expect(fileResult.status).toBe(1);
    expect(`${fileResult.stdout}${fileResult.stderr}`).toContain("build_output_directory_dirty");
    expect(await readFile(unknownFile, "utf8")).toBe("retain me\n");

    await rm(unknownFile);
    const unknownDirectory = join(dirty, "operator-data");
    await mkdir(unknownDirectory);
    const directoryResult = spawnSync(process.execPath, [buildScript.pathname, "--outdir", dirty], {
      encoding: "utf8",
    });
    expect(directoryResult.status).toBe(1);
    expect(`${directoryResult.stdout}${directoryResult.stderr}`).toContain(
      "build_output_directory_dirty",
    );
    expect(await readdir(dirty)).toContain("operator-data");
  });

  it("rejects a symlink or file destination without touching target bytes", async () => {
    // The build rejects pre-existing unsafe destinations. Concurrent hostile path replacement is
    // outside this local build contract; atomic publication belongs to deployment tooling.
    const target = join(temporaryRoot, "symlink-target");
    await mkdir(target);
    const sentinel = join(target, "suprnova-live.esm.js");
    await writeFile(sentinel, "target sentinel\n", "utf8");
    const linked = join(temporaryRoot, "linked-output");
    await symlink(target, linked, "dir");
    const linkedResult = spawnSync(process.execPath, [buildScript.pathname, "--outdir", linked], {
      encoding: "utf8",
    });
    expect(linkedResult.status).toBe(1);
    expect(`${linkedResult.stdout}${linkedResult.stderr}`).toContain(
      "build_output_directory_dirty",
    );
    expect(await readFile(sentinel, "utf8")).toBe("target sentinel\n");

    const directFile = join(temporaryRoot, "file-output");
    await writeFile(directFile, "direct sentinel\n", "utf8");
    const fileResult = spawnSync(process.execPath, [buildScript.pathname, "--outdir", directFile], {
      encoding: "utf8",
    });
    expect(fileResult.status).toBe(1);
    expect(`${fileResult.stdout}${fileResult.stderr}`).toContain("build_output_directory_dirty");
    expect(await readFile(directFile, "utf8")).toBe("direct sentinel\n");
  });
});
