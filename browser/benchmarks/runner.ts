import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { arch, cpus, platform, release, totalmem } from "node:os";
import { basename, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { brotliCompressSync, constants as zlibConstants } from "node:zlib";

import type { Browser, BrowserContext, CDPSession, Page } from "@playwright/test";
import { build } from "esbuild";

import {
  classifyBenchmarkEnvironment,
  validateBrowserBudgetResult,
  type BenchmarkEnvironment,
  type BrowserBudgetResult,
} from "./schema.js";
import { summarizeSamples } from "./statistics.js";
import { createD100Workload, createMorphWorkload, type MorphWorkload } from "./workloads.js";

const VIEWPORT = Object.freeze({ width: 1280, height: 720 });
const BENCHMARK_MORPH_DEADLINE_MS = 10_000;
const RUNTIME_ASSET = "/assets/suprnova-live.classic.js";
const browserRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

interface BrowserBudgetOptions {
  readonly browser: Browser;
  readonly context: BrowserContext;
  readonly baseUrl: string;
  readonly warmupSamples: number;
  readonly measuredSamples: number;
  readonly independentRuns: number;
  readonly idleDurationMs: number;
  readonly cpuThrottleRate: number;
  readonly dedicated: boolean;
}

interface MeasurementRun {
  readonly d100ConnectSamples: readonly number[];
  readonly m1kMorphSamples: readonly number[];
  readonly m5kMorphSamples: readonly number[];
}

interface IdleMeasurement {
  readonly mainThreadMs: number;
  readonly coreMutationObservers: number;
  readonly networkRequests: number;
  readonly pollingOperations: number;
}

interface InstrumentationState {
  coreMutationObservers: number;
  pollingOperations: number;
}

interface MeasurementObserverPort {
  mutation(callback: MutationCallback): MutationObserver;
  intersection(
    callback: IntersectionObserverCallback,
    options?: IntersectionObserverInit,
  ): IntersectionObserver | null;
}

interface RuntimeFacade {
  boot(options?: Readonly<Record<string, unknown>>): { stop(): void };
}

type BudgetWindow = Window &
  typeof globalThis & {
    SuprnovaLive: RuntimeFacade;
    __suprnovaBudgetInstrumentation: InstrumentationState;
    __suprnovaBudgetObservers: MeasurementObserverPort;
    __suprnovaBudgetHandle?: { stop(): void };
    __suprnovaBudgetMorph(
      input: Readonly<{
        html: string;
        authority: Readonly<{
          component: string;
          documentKey: string;
          encodedSnapshot: string;
          instanceId: string;
          slot: string;
          successorRevision: string;
        }>;
      }>,
    ): number;
  };

let cachedMorphHarness: Promise<string> | null = null;

function morphHarnessSource(): Promise<string> {
  cachedMorphHarness ??= build({
    absWorkingDir: browserRoot,
    bundle: true,
    charset: "utf8",
    format: "iife",
    legalComments: "none",
    minify: true,
    platform: "browser",
    stdin: {
      contents: `
        import { CompositionTracker, captureContinuity } from "./src/continuity/capture.js";
        import { restoreContinuity, restoreContinuityFocus } from "./src/continuity/restore.js";
        import { IdiomorphAdapter } from "./src/morph/idiomorph.js";
        import { DEFAULT_MORPH_LIMITS } from "./src/morph/limits.js";
        import { preflightIslandMorph } from "./src/morph/preflight.js";

        const adapter = new IdiomorphAdapter();
        window.__suprnovaBudgetMorph = ({ html, authority }) => {
          const currentRoot = document.querySelector("[data-suprnova-live-island]");
          if (!(currentRoot instanceof HTMLElement)) throw new Error("benchmark_root_missing");
          const started = performance.now();
          const plan = preflightIslandMorph({
            authority: {
              ...authority,
              successorRevision: BigInt(authority.successorRevision),
            },
            currentRoot,
            html,
              // Preserve production structural limits while allowing the benchmark to record a
              // slow result instead of aborting at the production interaction safety deadline.
              limits: { ...DEFAULT_MORPH_LIMITS, deadlineMs: ${String(BENCHMARK_MORPH_DEADLINE_MS)} },
          });
          const composition = new CompositionTracker(document);
          try {
            const continuity = captureContinuity(plan, {
              composition,
              signalScopes: [],
              stimulus: null,
            });
            const result = adapter.apply(plan, {
              beforeMorph() {},
              afterMorph() {},
              beforeNodeAdded() {},
              afterNodeAdded() {},
              beforeNodeMorphed() {},
              afterNodeMorphed() {},
              beforeNodeRemoved() {},
              afterNodeRemoved() {},
            });
            restoreContinuity(continuity, currentRoot, {
              restoreSignals: () => 0,
              stimulus: null,
            });
            restoreContinuityFocus(continuity, currentRoot);
            if (
              result.root.getAttribute("data-suprnova-live-revision") !== authority.successorRevision ||
              result.root.getAttribute("data-suprnova-live-snapshot") !== authority.encodedSnapshot
            ) {
              throw new Error("benchmark_commit_failed");
            }
            return performance.now() - started;
          } finally {
            composition.dispose();
          }
        };
      `,
      loader: "ts",
      resolveDir: browserRoot,
      sourcefile: "browser-budget-morph-port.ts",
    },
    target: ["chrome111"],
    treeShaking: true,
    write: false,
  }).then((result) => {
    const output = result.outputFiles[0];
    if (output === undefined) throw new Error("benchmark_harness_missing");
    return output.text;
  });
  return cachedMorphHarness;
}

function delay(milliseconds: number): Promise<void> {
  return new Promise((resolveDelay) => setTimeout(resolveDelay, milliseconds));
}

async function instrument(page: Page): Promise<void> {
  await page.evaluate(() => {
    const budgetWindow = window as BudgetWindow;
    const state: InstrumentationState = { coreMutationObservers: 0, pollingOperations: 0 };
    const nativeSetInterval = window.setInterval.bind(window);
    const countingSetInterval = (handler: TimerHandler, timeout?: number): number => {
      state.pollingOperations += 1;
      return nativeSetInterval(handler, timeout);
    };
    Object.defineProperty(window, "setInterval", {
      configurable: false,
      value: countingSetInterval,
      writable: false,
    });
    budgetWindow.__suprnovaBudgetInstrumentation = state;
    budgetWindow.__suprnovaBudgetObservers = Object.freeze({
      mutation: (callback: MutationCallback) => {
        state.coreMutationObservers += 1;
        return new MutationObserver(callback);
      },
      intersection: (callback: IntersectionObserverCallback, options?: IntersectionObserverInit) =>
        typeof IntersectionObserver === "undefined"
          ? null
          : new IntersectionObserver(callback, options),
    });
  });
}

async function cdpFor(page: Page, cpuThrottleRate: number): Promise<CDPSession> {
  const session = await page.context().newCDPSession(page);
  await session.send("Performance.enable");
  await session.send("Emulation.setCPUThrottlingRate", { rate: cpuThrottleRate });
  return session;
}

async function loadRuntime(page: Page, baseUrl: string): Promise<void> {
  await page.addScriptTag({ url: new URL(RUNTIME_ASSET, baseUrl).href });
}

async function connectedD100Page(
  context: BrowserContext,
  baseUrl: string,
  cpuThrottleRate: number,
): Promise<{ readonly page: Page; readonly session: CDPSession; readonly elapsedMs: number }> {
  const workload = createD100Workload();
  const page = await context.newPage();
  await page.setViewportSize(VIEWPORT);
  await page.goto(new URL("/health", baseUrl).href);
  await page.setContent(workload.html, { waitUntil: "domcontentloaded" });
  await instrument(page);
  const session = await cdpFor(page, cpuThrottleRate);
  await loadRuntime(page, baseUrl);
  const elapsedMs = await page.evaluate((expected) => {
    const budgetWindow = window as BudgetWindow;
    const started = performance.now();
    budgetWindow.__suprnovaBudgetHandle = budgetWindow.SuprnovaLive.boot({
      observers: budgetWindow.__suprnovaBudgetObservers,
    });
    const connected = document.querySelectorAll(
      '[data-suprnova-live-island][data-suprnova-live-status="connected"]',
    ).length;
    if (connected !== expected) throw new Error("d100_connection_failed");
    return performance.now() - started;
  }, workload.islandCount);
  return { page, session, elapsedMs };
}

async function measureD100Connect(
  context: BrowserContext,
  baseUrl: string,
  cpuThrottleRate: number,
  samples: number,
): Promise<readonly number[]> {
  const measured: number[] = [];
  for (let index = 0; index < samples; index += 1) {
    const { page, session, elapsedMs } = await connectedD100Page(context, baseUrl, cpuThrottleRate);
    measured.push(elapsedMs);
    await session.detach();
    await page.close();
  }
  return Object.freeze(measured);
}

async function measureMorphSample(
  context: BrowserContext,
  baseUrl: string,
  cpuThrottleRate: number,
  workload: MorphWorkload,
): Promise<number> {
  const page = await context.newPage();
  await page.setViewportSize(VIEWPORT);
  await page.goto(new URL("/health", baseUrl).href);
  await page.setContent(workload.sourceDocument, { waitUntil: "domcontentloaded" });
  await instrument(page);
  const session = await cdpFor(page, cpuThrottleRate);
  await page.addScriptTag({ content: await morphHarnessSource() });
  const elapsed = await page.evaluate(
    ({ authority, html }) => (window as BudgetWindow).__suprnovaBudgetMorph({ authority, html }),
    { authority: workload.targetAuthority, html: workload.targetHtml },
  );
  await session.detach();
  await page.close();
  return elapsed;
}

async function measureMorph(
  context: BrowserContext,
  baseUrl: string,
  cpuThrottleRate: number,
  workload: MorphWorkload,
  samples: number,
): Promise<readonly number[]> {
  const measured: number[] = [];
  for (let index = 0; index < samples; index += 1) {
    measured.push(await measureMorphSample(context, baseUrl, cpuThrottleRate, workload));
  }
  return Object.freeze(measured);
}

function taskDuration(metrics: Awaited<ReturnType<CDPSession["send"]>>): number {
  if (!("metrics" in metrics) || !Array.isArray(metrics.metrics)) {
    throw new Error("cdp_metrics_missing");
  }
  const task = metrics.metrics.find((metric) => metric.name === "TaskDuration");
  if (task === undefined) throw new Error("cdp_task_duration_missing");
  return task.value;
}

async function measureIdle(
  context: BrowserContext,
  baseUrl: string,
  cpuThrottleRate: number,
  idleDurationMs: number,
): Promise<IdleMeasurement> {
  const { page, session } = await connectedD100Page(context, baseUrl, cpuThrottleRate);
  let networkRequests = 0;
  const countRequest = (): void => {
    networkRequests += 1;
  };
  page.on("request", countRequest);
  await page.evaluate(() => {
    (window as BudgetWindow).__suprnovaBudgetInstrumentation.pollingOperations = 0;
  });
  const before = await session.send("Performance.getMetrics");
  await delay(idleDurationMs);
  const after = await session.send("Performance.getMetrics");
  const instrumentation = await page.evaluate(
    () => (window as BudgetWindow).__suprnovaBudgetInstrumentation,
  );
  page.off("request", countRequest);
  await session.detach();
  await page.close();
  return Object.freeze({
    mainThreadMs: Math.max(0, (taskDuration(after) - taskDuration(before)) * 1_000),
    coreMutationObservers: instrumentation.coreMutationObservers,
    networkRequests,
    pollingOperations: instrumentation.pollingOperations,
  });
}

async function heapUsage(
  context: BrowserContext,
  baseUrl: string,
  cpuThrottleRate: number,
  html: string,
  boot: boolean,
): Promise<number> {
  const page = await context.newPage();
  await page.setViewportSize(VIEWPORT);
  await page.goto(new URL("/health", baseUrl).href);
  await page.setContent(html, { waitUntil: "domcontentloaded" });
  await instrument(page);
  const session = await cdpFor(page, cpuThrottleRate);
  if (boot) {
    await loadRuntime(page, baseUrl);
    await page.evaluate(() => {
      const budgetWindow = window as BudgetWindow;
      budgetWindow.__suprnovaBudgetHandle = budgetWindow.SuprnovaLive.boot({
        observers: budgetWindow.__suprnovaBudgetObservers,
      });
    });
  }
  await session.send("HeapProfiler.collectGarbage");
  const usage = await session.send("Runtime.getHeapUsage");
  await session.detach();
  await page.close();
  return usage.usedSize;
}

async function retainedBytesPerIsland(
  context: BrowserContext,
  baseUrl: string,
  cpuThrottleRate: number,
): Promise<number> {
  const d100 = createD100Workload();
  const empty = createMorphWorkload("M1K").sourceDocument.replace(
    /<body>[\s\S]*<\/body>/u,
    "<body></body>",
  );
  const emptyControl = await heapUsage(context, baseUrl, cpuThrottleRate, empty, false);
  const emptyRuntime = await heapUsage(context, baseUrl, cpuThrottleRate, empty, true);
  const d100Control = await heapUsage(context, baseUrl, cpuThrottleRate, d100.html, false);
  const d100Runtime = await heapUsage(context, baseUrl, cpuThrottleRate, d100.html, true);
  const incremental = d100Runtime - d100Control - (emptyRuntime - emptyControl);
  return Math.max(0, incremental / d100.islandCount);
}

async function governor(): Promise<string> {
  try {
    return (await readFile("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor", "utf8")).trim();
  } catch {
    return "unavailable";
  }
}

async function environmentFor(
  browser: Browser,
  baseUrl: string,
  cpuThrottleRate: number,
  dedicated: boolean,
): Promise<BenchmarkEnvironment> {
  const packageJson = JSON.parse(
    await readFile(
      new URL("../node_modules/@playwright/test/package.json", import.meta.url),
      "utf8",
    ),
  ) as { version?: unknown };
  if (typeof packageJson.version !== "string") throw new Error("playwright_version_missing");
  const executableDirectory = basename(
    new URL(`file://${browser.browserType().executablePath()}`).pathname,
  );
  const executablePath = browser.browserType().executablePath();
  const revision = /(?:chromium|chrome)-([0-9]+)/u.exec(executablePath)?.[1] ?? executableDirectory;
  const cpu = cpus()[0];
  if (cpu === undefined) throw new Error("host_cpu_missing");
  const url = new URL(baseUrl);
  return Object.freeze({
    platform: platform(),
    architecture: arch(),
    kernel: release(),
    cpuModel: cpu.model,
    logicalCpuCount: cpus().length,
    memoryBytes: totalmem(),
    cpuGovernor: await governor(),
    dedicated,
    loopback:
      url.hostname === "127.0.0.1" || url.hostname === "localhost" || url.hostname === "[::1]",
    playwrightVersion: packageJson.version,
    browserName: "chromium",
    browserVersion: browser.version(),
    browserRevision: revision,
    viewport: VIEWPORT,
    cpuThrottleRate,
    extensions: false,
    warmHttpCache: true,
  });
}

async function artifact() {
  const bytes = await readFile(new URL("../dist/suprnova-live.esm.js", import.meta.url));
  return Object.freeze({
    file: "suprnova-live.esm.js" as const,
    sha256: createHash("sha256").update(bytes).digest("hex"),
    brotliBytes: brotliCompressSync(bytes, {
      params: { [zlibConstants.BROTLI_PARAM_QUALITY]: 11 },
    }).byteLength,
  });
}

async function warmRuntimeCache(context: BrowserContext, baseUrl: string): Promise<void> {
  const page = await context.newPage();
  await page.goto(new URL("/scenario/instance", baseUrl).href, { waitUntil: "networkidle" });
  await page.close();
}

async function measurementRun(
  context: BrowserContext,
  baseUrl: string,
  cpuThrottleRate: number,
  warmupSamples: number,
  measuredSamples: number,
): Promise<MeasurementRun> {
  const m1k = createMorphWorkload("M1K");
  const m5k = createMorphWorkload("M5K");
  await measureD100Connect(context, baseUrl, cpuThrottleRate, warmupSamples);
  await measureMorph(context, baseUrl, cpuThrottleRate, m1k, warmupSamples);
  await measureMorph(context, baseUrl, cpuThrottleRate, m5k, warmupSamples);
  return Object.freeze({
    d100ConnectSamples: await measureD100Connect(
      context,
      baseUrl,
      cpuThrottleRate,
      measuredSamples,
    ),
    m1kMorphSamples: await measureMorph(context, baseUrl, cpuThrottleRate, m1k, measuredSamples),
    m5kMorphSamples: await measureMorph(context, baseUrl, cpuThrottleRate, m5k, measuredSamples),
  });
}

export async function runBrowserBudget(
  options: BrowserBudgetOptions,
): Promise<BrowserBudgetResult> {
  if (
    !Number.isSafeInteger(options.warmupSamples) ||
    options.warmupSamples <= 0 ||
    !Number.isSafeInteger(options.measuredSamples) ||
    options.measuredSamples <= 0 ||
    options.measuredSamples > 100 ||
    !Number.isSafeInteger(options.independentRuns) ||
    options.independentRuns <= 0 ||
    options.independentRuns > 3 ||
    !Number.isSafeInteger(options.idleDurationMs) ||
    options.idleDurationMs <= 0
  ) {
    throw new Error("browser_budget_options_invalid");
  }
  await warmRuntimeCache(options.context, options.baseUrl);
  const runs: MeasurementRun[] = [];
  for (let index = 0; index < options.independentRuns; index += 1) {
    runs.push(
      await measurementRun(
        options.context,
        options.baseUrl,
        options.cpuThrottleRate,
        options.warmupSamples,
        options.measuredSamples,
      ),
    );
  }
  const connect = summarizeSamples(runs.flatMap((run) => run.d100ConnectSamples));
  const m1kMorph = summarizeSamples(runs.flatMap((run) => run.m1kMorphSamples));
  const m5kMorph = summarizeSamples(runs.flatMap((run) => run.m5kMorphSamples));
  const idle = await measureIdle(
    options.context,
    options.baseUrl,
    options.cpuThrottleRate,
    options.idleDurationMs,
  );
  const environment = await environmentFor(
    options.browser,
    options.baseUrl,
    options.cpuThrottleRate,
    options.dedicated,
  );
  const m1k = createMorphWorkload("M1K");
  const m5k = createMorphWorkload("M5K");
  return validateBrowserBudgetResult({
    schemaVersion: 1,
    classification: classifyBenchmarkEnvironment(environment),
    recordedAt: new Date().toISOString(),
    artifact: await artifact(),
    environment,
    methodology: {
      warmupSamples: options.warmupSamples,
      measuredSamples: options.measuredSamples,
      independentRuns: options.independentRuns,
      idleDurationMs: options.idleDurationMs,
      retainedMemory: "d100-minus-control-minus-fixed-runtime-v1",
      mainThreadTime: "cdp-performance-task-duration-v1",
      observerCount: "instrumented-runtime-observer-factory-v1",
      morphMeasurement: "bundled-production-morph-port-v1",
      morphDeadlineMs: BENCHMARK_MORPH_DEADLINE_MS,
      correctnessEnabled: true,
      accessibilityEnabled: true,
      lifecycleEnabled: true,
    },
    workloads: {
      D100: {
        documentBytes: 65_536,
        islandCount: 100,
        connect,
        idleMainThreadMs: idle.mainThreadMs,
        coreMutationObservers: idle.coreMutationObservers,
        idleNetworkRequests: idle.networkRequests,
        idlePollingOperations: idle.pollingOperations,
        retainedBytesPerIsland: await retainedBytesPerIsland(
          options.context,
          options.baseUrl,
          options.cpuThrottleRate,
        ),
      },
      M1K: {
        elementCount: m1k.elementCount,
        maximumDepth: m1k.maximumDepth,
        changedNodeCount: m1k.changedNodeCount,
        morph: m1kMorph,
      },
      M5K: {
        elementCount: m5k.elementCount,
        maximumDepth: m5k.maximumDepth,
        changedNodeCount: m5k.changedNodeCount,
        morph: m5kMorph,
      },
    },
    independentP95Ms: {
      d100Connect: runs.map((run) => summarizeSamples(run.d100ConnectSamples).p95Ms),
      m1kMorph: runs.map((run) => summarizeSamples(run.m1kMorphSamples).p95Ms),
      m5kMorph: runs.map((run) => summarizeSamples(run.m5kMorphSamples).p95Ms),
    },
  });
}
