import { describe, expect, it, vi, type Mock } from "vitest";

import type { JsonValue } from "../src/canonical.js";
import {
  createOptionalFeatureDriver,
  defineAsyncFeature,
  defineUploadsFeature,
  sealValidatedAsyncDescriptor,
  type AsyncRuntimeIslandPort,
  RUNTIME_FEATURE_CORE_RANGE,
  RUNTIME_STIMULUS_ADAPTER_FORMAT,
  RUNTIME_STIMULUS_ADAPTER_IDENTITY,
  type FeatureDocumentController,
  type FeatureIslandController,
  type RuntimeFeature,
  type RuntimeFeatureDefinition,
  type RuntimeFeatureDocumentContext,
  type RuntimeFeatureIslandPort,
  type RuntimeFeatureName,
  type RuntimeStimulusAdapter,
  type UploadsRuntimeIslandPort,
} from "../src/features/contract.js";
import { CLASSIC_FEATURE_SYMBOL, adoptClassicFeatures } from "../src/features/global.js";
import {
  registerClassicFeature,
  registerRuntimeFeature,
  registerRuntimeStimulusAdapter,
} from "../src/features/producer.js";
import { installStimulusAdapter } from "../src/features/stimulus.js";
import {
  RUNTIME_FEATURE_DRIVER_CORE_RANGE,
  RUNTIME_FEATURE_DRIVER_FORMAT,
  inspectRuntimeFeatureDriver,
  type FreshRenderDisposition,
  type FreshRenderCompletionObserver,
  type FreshRenderReason,
  type RuntimeFeatureDiagnosticDetail,
  type RuntimeFeatureDriver,
  type InspectedRuntimeFeatureDriver,
  type RuntimeFeatureDriverDocumentPort,
  type RuntimeFeatureDriverIslandPort,
  type RuntimeFeatureDriverRegistrationHost,
  type RuntimeFeatureDriverValue,
  type RuntimeFeatureRegistrationOutcome,
  type RegisteredBrowserEventCapability,
  type RegisteredBrowserEventDispatch,
  type RegisteredBrowserEventDisposition,
  type RegisteredBrowserEventRegistration,
} from "../src/features/host.js";
import { MAX_PRESENT_DIRECTIVES } from "../src/directives/parser.js";
import { parseFeatureDirective } from "../src/features/directive-parser.js";
import { IslandRecord } from "../src/islands/record.js";
import type { IslandMetadata } from "../src/islands/metadata.js";
import type { StimulusBootstrapOptions } from "../src/stimulus/port.js";
import type {
  UploadHandleProposal,
  UploadHandleProposalDisposition,
} from "../src/uploads/types.js";

interface Counters {
  readonly abortMorph: Mock<() => void>;
  readonly afterMorph: Mock<() => void>;
  readonly beforeMorph: Mock<() => void>;
  readonly connectDocument: Mock<(context: RuntimeFeatureDocumentContext) => void>;
  readonly connectIsland: Mock<(port: RuntimeFeatureIslandPort) => void>;
  readonly disposeDocument: Mock<() => void>;
  readonly disposeIsland: Mock<() => void>;
  readonly resumeDocument: Mock<() => void>;
  readonly resumeIsland: Mock<() => void>;
  readonly suspendDocument: Mock<() => void>;
  readonly suspendIsland: Mock<() => void>;
}

interface FeatureFixture {
  readonly counters: Counters;
  readonly feature: RuntimeFeature;
  readonly documentContexts: RuntimeFeatureDocumentContext[];
  readonly islandPorts: RuntimeFeatureIslandPort[];
}

function feature(
  name: RuntimeFeatureName,
  overrides: Partial<RuntimeFeatureDefinition> = {},
): FeatureFixture {
  const documentContexts: RuntimeFeatureDocumentContext[] = [];
  const islandPorts: RuntimeFeatureIslandPort[] = [];
  const counters: Counters = {
    abortMorph: vi.fn<() => void>(),
    afterMorph: vi.fn<() => void>(),
    beforeMorph: vi.fn<() => void>(),
    connectDocument: vi.fn<(context: RuntimeFeatureDocumentContext) => void>(),
    connectIsland: vi.fn<(port: RuntimeFeatureIslandPort) => void>(),
    disposeDocument: vi.fn<() => void>(),
    disposeIsland: vi.fn<() => void>(),
    resumeDocument: vi.fn<() => void>(),
    resumeIsland: vi.fn<() => void>(),
    suspendDocument: vi.fn<() => void>(),
    suspendIsland: vi.fn<() => void>(),
  };
  const definition: RuntimeFeatureDefinition = {
    connectDocument(context) {
      documentContexts.push(context);
      counters.connectDocument(context);
      const controller: FeatureDocumentController = {
        connectIsland(port) {
          islandPorts.push(port);
          counters.connectIsland(port);
          const island: FeatureIslandController = {
            abortMorph: counters.abortMorph,
            afterMorph: counters.afterMorph,
            beforeMorph: counters.beforeMorph,
            dispose: counters.disposeIsland,
            resume: counters.resumeIsland,
            suspend: counters.suspendIsland,
          };
          return island;
        },
        dispose: counters.disposeDocument,
        resume: counters.resumeDocument,
        suspend: counters.suspendDocument,
      };
      return controller;
    },
    ...overrides,
  };
  return {
    counters,
    documentContexts,
    feature: name === "uploads" ? defineUploadsFeature(definition) : defineAsyncFeature(definition),
    islandPorts,
  };
}

interface DriverIslandSource {
  readonly authorizeRegisteredEvents: Mock<
    (registration: RegisteredBrowserEventRegistration) => RegisteredBrowserEventCapability
  >;
  readonly dispatchRegisteredEvent: Mock<
    (
      capability: RegisteredBrowserEventCapability,
      event: RegisteredBrowserEventDispatch,
    ) => RegisteredBrowserEventDisposition
  >;
  readonly element: Element;
  readonly identity: Readonly<{ component: string; documentKey: string; slot: string }>;
  readonly enqueueFreshRender: Mock<
    (
      reason: FreshRenderReason,
      completion?: FreshRenderCompletionObserver,
      completionKey?: string,
    ) => FreshRenderDisposition
  >;
  readonly proposeUploadHandle: Mock<
    (field: string, proposal: UploadHandleProposal) => UploadHandleProposalDisposition
  >;
  readonly writePresentationSignal: Mock<
    (scope: string, name: string, value: JsonValue) => JsonValue
  >;
  active(): boolean;
  retire(): void;
}

function islandSource(name: string, element?: Element): DriverIslandSource {
  let active = true;
  const capability = Object.freeze({}) as RegisteredBrowserEventCapability;
  return {
    active: () => active,
    authorizeRegisteredEvents: vi.fn(() => capability),
    dispatchRegisteredEvent: vi.fn(() => "dispatched" as const),
    element: element ?? ({ nodeType: 1, setAttribute: vi.fn() } as unknown as Element),
    enqueueFreshRender: vi.fn(() => "queued"),
    identity: Object.freeze({
      component: "fixture.search",
      documentKey: `document-${name}`,
      slot: `slot-${name}`,
    }),
    proposeUploadHandle: vi.fn(() => "accepted"),
    retire() {
      active = false;
    },
    writePresentationSignal: vi.fn((_scope, _signal, value) => value),
  };
}

type DriverState = "idle" | "active" | "suspended" | "retired";
interface ClaimedIsland {
  claimed: boolean;
  readonly source: DriverIslandSource;
}

class DriverRuntime implements RuntimeFeatureDriverRegistrationHost {
  readonly diagnostics: RuntimeFeatureDiagnosticDetail[] = [];
  readonly islands = new Map<Element, ClaimedIsland>();
  #driver: InspectedRuntimeFeatureDriver | null = null;
  #ready = false;
  #started = false;
  #state: DriverState = "idle";

  constructor(readonly stimulus?: StimulusBootstrapOptions) {}

  register(driver: RuntimeFeatureDriver): RuntimeFeatureRegistrationOutcome {
    if (this.#state === "retired") return "incompatible";
    const inspected = inspectRuntimeFeatureDriver(driver);
    if (inspected === null) return "incompatible";
    if (this.#driver !== null) return this.#driver === driver ? "already_registered" : "conflict";
    this.#driver = inspected;
    if (this.#state === "active") this.#startDriver();
    return "registered";
  }

  size(): number {
    return this.#driver === null ? 0 : 1;
  }

  start(): void {
    if (this.#state !== "idle") return;
    this.#state = "active";
    this.#startDriver();
  }

  connectIsland(source: DriverIslandSource): "connected" | "already_connected" | "retired" {
    if (this.#state === "retired") return "retired";
    if (this.islands.has(source.element)) return "already_connected";
    const island = { claimed: false, source };
    this.islands.set(source.element, island);
    this.#connectDriver(island);
    return "connected";
  }

  retireIsland(element: Element): void {
    const island = this.islands.get(element);
    if (island === undefined) return;
    this.islands.delete(element);
    island.source.retire();
    if (!island.claimed) return;
    island.claimed = false;
    this.#invoke(4, element);
  }

  suspend(): void {
    if (this.#state !== "active") return;
    this.#state = "suspended";
    if (this.#ready) this.#invoke(2, null);
  }

  resume(): void {
    if (this.#state !== "suspended") return;
    this.#state = "active";
    if (this.#ready) this.#invoke(3, null);
    else this.#startDriver();
  }

  dispose(): void {
    if (this.#state === "retired") return;
    this.#state = "retired";
    for (const element of [...this.islands.keys()]) this.retireIsland(element);
    const driver = this.#driver;
    const started = this.#started;
    this.#driver = null;
    this.#ready = false;
    this.#started = false;
    if (driver !== null && started) this.#invokeEntry(driver, 5, null);
  }

  #startDriver(): void {
    const driver = this.#driver;
    if (driver === null || this.#state !== "active" || this.#started) return;
    this.#started = true;
    const port: RuntimeFeatureDriverDocumentPort = Object.freeze({
      diagnose: (detail: RuntimeFeatureDiagnosticDetail) => {
        this.diagnostics.push(detail);
      },
      ...(this.stimulus === undefined ? {} : { stimulus: this.stimulus }),
    });
    if (!this.#invokeEntry(driver, 0, port)) return;
    if (!this.#current(driver)) return;
    this.#ready = true;
    for (const island of this.islands.values()) this.#connectDriver(island);
  }

  #connectDriver(island: ClaimedIsland): void {
    const driver = this.#driver;
    if (
      driver === null ||
      !this.#ready ||
      this.#state !== "active" ||
      !island.source.active() ||
      island.claimed
    ) {
      return;
    }
    island.claimed = true;
    const current = (): boolean =>
      this.#current(driver) && island.claimed && island.source.active();
    const port: RuntimeFeatureDriverIslandPort = Object.freeze({
      authorizeRegisteredEvents: (registration: RegisteredBrowserEventRegistration) => {
        if (!current()) throw new Error("stale_driver_port");
        return island.source.authorizeRegisteredEvents(registration);
      },
      dispatchRegisteredEvent: (
        capability: RegisteredBrowserEventCapability,
        event: RegisteredBrowserEventDispatch,
      ) => {
        if (!current()) return "retired";
        return island.source.dispatchRegisteredEvent(capability, event);
      },
      element: island.source.element,
      enqueueFreshRender: (
        reason: FreshRenderReason,
        completion?: FreshRenderCompletionObserver,
        completionKey?: string,
      ) => {
        const candidate: unknown = reason;
        if (!current() || (candidate !== "poll" && candidate !== "stream")) {
          completion?.("retired");
          return "retired";
        }
        if (completion === undefined) return island.source.enqueueFreshRender(candidate);
        return completionKey === undefined
          ? island.source.enqueueFreshRender(candidate, completion)
          : island.source.enqueueFreshRender(candidate, completion, completionKey);
      },
      identity: Object.freeze({ ...island.source.identity }),
      proposeUploadHandle: (field: string, proposal: UploadHandleProposal) => {
        if (!current()) return "retired";
        return island.source.proposeUploadHandle(field, proposal);
      },
      writePresentationSignal: (scope: string, name: string, value: JsonValue) => {
        if (!current()) throw new Error("stale_driver_port");
        return island.source.writePresentationSignal(scope, name, value);
      },
    });
    this.#invokeEntry(driver, 1, port);
  }

  #invoke(event: 0 | 1 | 2 | 3 | 4 | 5, value: RuntimeFeatureDriverValue): boolean {
    return this.#driver === null ? false : this.#invokeEntry(this.#driver, event, value);
  }

  morph(event: 6 | 7 | 8, scope: Element): boolean {
    return this.#driver === null ? true : this.#invokeEntry(this.#driver, event, scope);
  }

  #current(driver: InspectedRuntimeFeatureDriver): boolean {
    return this.#state === "active" && this.#driver === driver;
  }

  #invokeEntry(
    driver: InspectedRuntimeFeatureDriver,
    event: 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8,
    value: RuntimeFeatureDriverValue,
  ): boolean {
    try {
      const result: unknown = Reflect.apply(driver[4], driver, [event, value]);
      return result === true;
    } catch {
      this.diagnostics.push("operation_rejected");
      return false;
    }
  }
}

class FeatureRuntime {
  readonly target = Object.create(null) as typeof globalThis;
  readonly driver: DriverRuntime;

  constructor(stimulus?: StimulusBootstrapOptions) {
    this.driver = new DriverRuntime(stimulus);
    Object.defineProperty(this.target, Symbol.for("suprnova.live.runtime.v1"), {
      value: {
        register: (driver: RuntimeFeatureDriver) => this.driver.register(driver),
        status: () => "running",
        stop: () => undefined,
      },
    });
  }

  register(candidate: RuntimeFeature): RuntimeFeatureRegistrationOutcome {
    const outcome = registerRuntimeFeature(this.target, candidate);
    adoptClassicFeatures(this.target, this.driver);
    return outcome;
  }

  size(): number {
    return this.driver.size();
  }

  start(): void {
    this.driver.start();
  }

  connectIsland(source: DriverIslandSource): ReturnType<DriverRuntime["connectIsland"]> {
    return this.driver.connectIsland(source);
  }

  retireIsland(element: Element): void {
    this.driver.retireIsland(element);
  }

  suspend(): void {
    this.driver.suspend();
  }

  resume(): void {
    this.driver.resume();
  }

  dispose(): void {
    this.driver.dispose();
  }
}

describe("closed optional feature registration", () => {
  it("attaches one exact driver and keeps feature identity idempotence optional-side", () => {
    const runtime = new FeatureRuntime();
    const uploads = feature("uploads");
    const conflicting = feature("uploads");

    expect(runtime.register(uploads.feature)).toBe("registered");
    expect(runtime.register(uploads.feature)).toBe("already_registered");
    expect(runtime.register(conflicting.feature)).toBe("conflict");
    expect(runtime.size()).toBe(1);
    runtime.start();

    expect(uploads.counters.connectDocument).toHaveBeenCalledOnce();
    expect(conflicting.counters.connectDocument).not.toHaveBeenCalled();
  });

  it("rejects malformed feature slots, capability versions, ranges, and callbacks", () => {
    const runtime = new FeatureRuntime();
    const uploads = feature("uploads").feature;
    const forge = (index: number, value: unknown): RuntimeFeature => {
      const values: unknown[] = [...uploads];
      values[index] = value;
      return Object.freeze(values) as unknown as RuntimeFeature;
    };

    expect(runtime.register(forge(1, 2))).toBe("incompatible");
    expect(runtime.register(forge(2, 2))).toBe("incompatible");
    expect(runtime.register(forge(3, RUNTIME_FEATURE_CORE_RANGE - 1))).toBe("incompatible");
    expect(runtime.register(forge(5, "not-a-driver"))).toBe("incompatible");
  });

  it("rejects brand-only, wrong-version, wrong-range, and accessor-forged driver envelopes", () => {
    const drive = vi.fn(() => true);
    const valid = Object.freeze([
      RUNTIME_FEATURE_DRIVER_FORMAT,
      1,
      RUNTIME_FEATURE_DRIVER_CORE_RANGE,
      Object.freeze({}),
      drive,
    ]) as unknown as RuntimeFeatureDriver;
    const forge = (index: number, value: unknown): RuntimeFeatureDriver => {
      const candidate: unknown[] = [...valid];
      candidate[index] = value;
      return Object.freeze(candidate) as unknown as RuntimeFeatureDriver;
    };
    const accessor: unknown[] = [...valid];
    const getter = vi.fn(() => drive);
    Object.defineProperty(accessor, 4, { enumerable: true, get: getter });
    Object.freeze(accessor);

    expect(inspectRuntimeFeatureDriver(valid)).not.toBeNull();
    expect(inspectRuntimeFeatureDriver(forge(1, 2))).toBeNull();
    expect(inspectRuntimeFeatureDriver(forge(2, RUNTIME_FEATURE_DRIVER_CORE_RANGE - 1))).toBeNull();
    expect(inspectRuntimeFeatureDriver(Object.freeze([RUNTIME_FEATURE_DRIVER_FORMAT]))).toBeNull();
    expect(inspectRuntimeFeatureDriver(accessor)).toBeNull();
    expect(getter).not.toHaveBeenCalled();
  });

  it("allows one driver attachment, repeats the same object, and conflicts another identity", () => {
    const firstTarget = Object.create(null) as typeof globalThis;
    const secondTarget = Object.create(null) as typeof globalThis;
    const runtime = new DriverRuntime();
    registerRuntimeFeature(firstTarget, feature("uploads").feature);
    registerRuntimeFeature(secondTarget, feature("async").feature);

    adoptClassicFeatures(firstTarget, runtime);
    expect(runtime.size()).toBe(1);
    adoptClassicFeatures(firstTarget, runtime);
    expect(runtime.size()).toBe(1);
    adoptClassicFeatures(secondTarget, runtime);
    expect(runtime.size()).toBe(1);
  });

  it("reads a hostile producer definition once and redacts accessor failure", () => {
    const reads = vi.fn();
    const hostile = new Proxy(Object.create(null) as RuntimeFeatureDefinition, {
      get(_target, property) {
        if (property === "connectDocument") {
          reads();
          throw new Error("secret-accessor-sentinel");
        }
        return undefined;
      },
    });

    expect(() => defineUploadsFeature(hostile)).toThrow("feature_controller_invalid");
    expect(reads).toHaveBeenCalledOnce();
  });
});

describe("one driver claim and optional owner per island", () => {
  it("unwinds document disposers immediately when connectDocument throws", () => {
    const dispose = vi.fn();
    const runtime = new FeatureRuntime();
    runtime.register(
      defineUploadsFeature({
        connectDocument(context) {
          context.onDispose(dispose);
          throw new Error("secret-document-connect");
        },
      }),
    );

    runtime.start();
    expect(dispose).toHaveBeenCalledOnce();
    runtime.dispose();
    expect(dispose).toHaveBeenCalledOnce();
  });

  it("unwinds a partial document controller when normalization fails", () => {
    const disposeContext = vi.fn();
    const disposeController = vi.fn();
    const runtime = new FeatureRuntime();
    runtime.register(
      defineUploadsFeature({
        connectDocument(context) {
          context.onDispose(disposeContext);
          return {
            dispose: disposeController,
            get connectIsland(): FeatureDocumentController["connectIsland"] {
              throw new Error("secret-document-normalization");
            },
          };
        },
      }),
    );

    runtime.start();
    expect(disposeContext).toHaveBeenCalledOnce();
    expect(disposeController).toHaveBeenCalledOnce();
    runtime.dispose();
    expect(disposeContext).toHaveBeenCalledOnce();
    expect(disposeController).toHaveBeenCalledOnce();
  });

  it("unwinds island disposers immediately when connectIsland throws", () => {
    const dispose = vi.fn();
    const runtime = new FeatureRuntime();
    runtime.register(
      defineUploadsFeature({
        connectDocument() {
          return {
            connectIsland(port) {
              port.onDispose(dispose);
              throw new Error("secret-island-connect");
            },
            dispose: vi.fn(),
          };
        },
      }),
    );
    runtime.start();
    const island = islandSource("throwing-island");

    runtime.connectIsland(island);
    expect(dispose).toHaveBeenCalledOnce();
    runtime.retireIsland(island.element);
    runtime.dispose();
    expect(dispose).toHaveBeenCalledOnce();
  });

  it("unwinds a partial island controller when normalization fails", () => {
    const disposePort = vi.fn();
    const disposeController = vi.fn();
    const runtime = new FeatureRuntime();
    runtime.register(
      defineUploadsFeature({
        connectDocument() {
          return {
            connectIsland(port) {
              port.onDispose(disposePort);
              return {
                dispose: disposeController,
                get resume(): VoidFunction {
                  throw new Error("secret-island-normalization");
                },
              };
            },
            dispose: vi.fn(),
          };
        },
      }),
    );
    runtime.start();
    const island = islandSource("invalid-island-controller");

    runtime.connectIsland(island);
    expect(disposePort).toHaveBeenCalledOnce();
    expect(disposeController).toHaveBeenCalledOnce();
    runtime.retireIsland(island.element);
    runtime.dispose();
    expect(disposePort).toHaveBeenCalledOnce();
    expect(disposeController).toHaveBeenCalledOnce();
  });

  it("coalesces feature refreshes through the ordinary island scheduler", () => {
    const element = { setAttribute: vi.fn() } as unknown as Element;
    const record = new IslandRecord(element, Object.create(null) as IslandMetadata, 2, 1);

    expect(record.enqueueFreshRender("poll")).toBe("queued");
    expect(record.enqueueFreshRender("stream")).toBe("coalesced");
    expect(record.scheduler.snapshot()).toMatchObject({ inFlight: 0, queued: 1 });
    record.dispose();
    expect(record.enqueueFreshRender("poll")).toBe("retired");
  });

  it("preserves bounded semantic completion ownership through the production async port", () => {
    const runtime = new FeatureRuntime();
    const asynchronous = feature("async");
    const source = islandSource("async-completion-owner");
    runtime.register(asynchronous.feature);
    runtime.start();
    runtime.connectIsland(source);
    const port = asynchronous.islandPorts[0] as AsyncRuntimeIslandPort | undefined;
    if (port === undefined) throw new Error("async port fixture missing");
    const completion = vi.fn<FreshRenderCompletionObserver>();

    for (let index = 0; index < 1_000; index += 1) {
      expect(port.enqueueFreshRender("stream", completion, "subscription-orders")).toBe("queued");
    }

    expect(source.enqueueFreshRender).toHaveBeenCalledTimes(1_000);
    expect(source.enqueueFreshRender).toHaveBeenLastCalledWith(
      "stream",
      completion,
      "subscription-orders",
    );
  });

  it("scans feature directives optional-side and excludes nested islands", () => {
    function element(
      attributes: readonly Readonly<{ name: string; value: string }>[],
      children: Element[] = [],
      island = false,
    ): Element {
      return {
        attributes,
        children,
        matches: () => island,
        nodeType: 1,
        shadowRoot: null,
      } as unknown as Element;
    }
    const nestedUpload = element([{ name: "live:upload", value: "nested" }], [], true);
    const upload = element([{ name: "live:upload", value: "avatar" }]);
    const asynchronous = element([{ name: "live:stream", value: "orders" }]);
    const root = element([], [upload, asynchronous, nestedUpload], true);
    const source = islandSource("directives", root);
    const runtime = new FeatureRuntime();
    const uploads = feature("uploads");
    runtime.register(uploads.feature);
    runtime.start();
    runtime.connectIsland(source);

    expect(uploads.islandPorts[0]?.queryDirectiveOwnership(parseFeatureDirective)).toEqual([
      expect.objectContaining({ attributeName: "live:upload", element: upload }),
    ]);
  });

  it("fails closed after bounded hostile attribute inspection", () => {
    let inspected = 0;
    const attributes = {
      *[Symbol.iterator]() {
        for (let index = 0; index < 10_000; index += 1) {
          inspected += 1;
          yield {
            name: index === 0 ? "live:upload" : `data-hostile-${String(index)}`,
            value: index === 0 ? "avatar" : "sentinel",
          };
        }
      },
    } as unknown as Element["attributes"];
    const root = {
      attributes,
      children: [],
      matches: () => true,
      nodeType: 1,
      shadowRoot: null,
    } as unknown as Element;
    const runtime = new FeatureRuntime();
    const uploads = feature("uploads");
    runtime.register(uploads.feature);
    runtime.start();
    runtime.connectIsland(islandSource("hostile-attributes", root));

    expect(uploads.islandPorts[0]?.queryDirectiveOwnership(parseFeatureDirective)).toEqual([]);
    expect(inspected).toBe(MAX_PRESENT_DIRECTIVES + 1);
    expect(runtime.driver.diagnostics).toEqual(["resource_exhausted"]);
  });

  it("connects existing, dynamic, and late-second-slot islands exactly once", () => {
    const runtime = new FeatureRuntime();
    const first = islandSource("first");
    const late = islandSource("late");
    const uploads = feature("uploads");
    const asynchronous = feature("async");

    runtime.connectIsland(first);
    runtime.register(uploads.feature);
    runtime.start();
    runtime.connectIsland(late);
    expect(runtime.connectIsland(late)).toBe("already_connected");
    runtime.register(asynchronous.feature);

    expect(uploads.counters.connectIsland).toHaveBeenCalledTimes(2);
    expect(asynchronous.counters.connectIsland).toHaveBeenCalledTimes(2);
  });

  it("bounds optional island ownership at 256 and frees capacity on retirement", () => {
    const runtime = new FeatureRuntime();
    const uploads = feature("uploads");
    runtime.register(uploads.feature);
    runtime.start();
    const admitted = Array.from({ length: 256 }, (_, index) =>
      islandSource(`bounded-${String(index)}`),
    );
    for (const island of admitted) expect(runtime.connectIsland(island)).toBe("connected");

    const overflow = islandSource("bounded-overflow");
    expect(runtime.connectIsland(overflow)).toBe("connected");
    expect(uploads.counters.connectIsland).toHaveBeenCalledTimes(256);
    expect(runtime.driver.diagnostics).toEqual(["resource_exhausted"]);

    const uploadPort = uploads.islandPorts[0] as UploadsRuntimeIslandPort | undefined;
    expect(uploadPort?.proposeUploadHandle("avatar", null)).toBe("accepted");
    expect(admitted[0]?.proposeUploadHandle).toHaveBeenCalledWith("avatar", null);
    const retired = admitted[0];
    if (retired === undefined) throw new Error("bounded island fixture missing");
    runtime.retireIsland(retired.element);

    expect(runtime.connectIsland(islandSource("bounded-replacement"))).toBe("connected");
    expect(uploads.counters.connectIsland).toHaveBeenCalledTimes(257);
    expect(uploads.counters.disposeIsland).toHaveBeenCalledOnce();
    expect(runtime.driver.diagnostics).toEqual(["resource_exhausted"]);
  });

  it("gives each optional artifact only its slot-specific productive runtime effects", () => {
    const runtime = new FeatureRuntime();
    const uploads = feature("uploads");
    const asynchronous = feature("async");
    const uploadSource = islandSource("upload-surface");
    const asyncSource = islandSource("async-surface");
    runtime.register(uploads.feature);
    runtime.register(asynchronous.feature);
    runtime.start();
    runtime.connectIsland(uploadSource);
    runtime.connectIsland(asyncSource);

    const documentContext = uploads.documentContexts[0];
    const uploadPort = uploads.islandPorts[0] as UploadsRuntimeIslandPort | undefined;
    const asyncPort = asynchronous.islandPorts[1] as AsyncRuntimeIslandPort | undefined;
    expect(Object.keys(documentContext ?? {}).sort()).toEqual(["diagnose", "onDispose"]);
    expect(Object.keys(uploadPort ?? {}).sort()).toEqual([
      "element",
      "identity",
      "onDispose",
      "proposeUploadHandle",
      "queryDirectiveOwnership",
    ]);
    expect(Object.keys(asyncPort ?? {}).sort()).toEqual([
      "consumeRegisteredEventCapability",
      "dispatchRegisteredEvent",
      "element",
      "enqueueFreshRender",
      "identity",
      "onDispose",
      "projectAsyncStatus",
      "queryDirectiveOwnership",
      "writePresentationSignal",
    ]);
    for (const forbidden of [
      "enqueueAction",
      "invokeEffect",
      "installHtml",
      "mutateSnapshot",
      "writeAuthority",
      "commitResponse",
      "dispatchJavascript",
      "endpoint",
      "module",
    ]) {
      expect(forbidden in (documentContext ?? {})).toBe(false);
      expect(forbidden in (uploadPort ?? {})).toBe(false);
      expect(forbidden in (asyncPort ?? {})).toBe(false);
    }
    expect(Reflect.get(asyncPort ?? {}, "authorizeRegisteredEvents")).toBeUndefined();
    expect(Reflect.get(asyncPort ?? {}, "proposeUploadHandle")).toBeUndefined();
    expect(Reflect.get(asyncPort ?? {}, "writeState")).toBeUndefined();
    if (asyncPort !== undefined) {
      // @ts-expect-error -- the async slot must not expose upload authority in declarations.
      void asyncPort.proposeUploadHandle;
      // @ts-expect-error -- the async slot consumes only a core-minted current capability.
      void asyncPort.authorizeRegisteredEvents;
    }
    for (const forbidden of [
      "authorizeRegisteredEvents",
      "proposeUploadHandle",
      "registerEvent",
      "registerEvents",
      "writeModel",
      "writeState",
    ]) {
      expect(forbidden in (asyncPort ?? {})).toBe(false);
    }
    for (const forbidden of [
      "consumeRegisteredEventCapability",
      "dispatchRegisteredEvent",
      "enqueueFreshRender",
      "writePresentationSignal",
    ]) {
      expect(forbidden in (uploadPort ?? {})).toBe(false);
    }
    expect(Object.isFrozen(asyncPort?.identity)).toBe(true);
    expect(asyncPort?.enqueueFreshRender("stream")).toBe("queued");
    const freshRenderCompletion = vi.fn<FreshRenderCompletionObserver>();
    expect(asyncPort?.enqueueFreshRender("stream", freshRenderCompletion)).toBe("queued");
    expect(asyncSource.enqueueFreshRender).toHaveBeenLastCalledWith(
      "stream",
      freshRenderCompletion,
    );
    expect(uploadPort?.proposeUploadHandle("avatar", "018f47c1-2af0-7cc4-a001-000000000001")).toBe(
      "accepted",
    );
    expect(uploadSource.proposeUploadHandle).toHaveBeenCalledWith(
      "avatar",
      "018f47c1-2af0-7cc4-a001-000000000001",
    );
    expect(asyncPort?.writePresentationSignal("root-scope", "progress", 42)).toBe(42);
    if (asyncPort === undefined) throw new Error("async port fixture missing");
    const descriptor = {
      descriptorBinding: "binding-v1",
      events: [
        {
          cycle: { kind: "forbid_repeated_island" },
          maximumFanout: 1,
          name: "orders.updated",
          order: "per_source_sequence",
          payloadContract: "orders.updated.v1",
          schema: "json",
          source: "stream",
          targets: ["self"],
          version: 1,
        },
      ],
    } as never;
    expect(() => asyncPort.consumeRegisteredEventCapability(descriptor)).toThrow(
      "async_descriptor_capability_invalid",
    );
    expect(asyncSource.authorizeRegisteredEvents).not.toHaveBeenCalled();
    const descriptorCapability = sealValidatedAsyncDescriptor(asyncPort, descriptor);
    const eventCapability = asyncPort.consumeRegisteredEventCapability(descriptorCapability);
    expect(() => asyncPort.consumeRegisteredEventCapability(descriptorCapability)).toThrow(
      "async_descriptor_capability_invalid",
    );
    expect(
      Reflect.apply(
        Reflect.get(asyncPort, "dispatchRegisteredEvent") as (...values: unknown[]) => unknown,
        asyncPort,
        [
          eventCapability,
          Object.freeze({
            event: "orders.updated",
            payload: Object.freeze({ count: 1 }),
            schemaVersion: 1,
            target: "self",
          }),
        ],
      ),
    ).toBe("dispatched");
    expect(asyncSource.authorizeRegisteredEvents).toHaveBeenCalledOnce();
    expect(asyncSource.dispatchRegisteredEvent).toHaveBeenCalledOnce();
    expect(uploadSource.authorizeRegisteredEvents).not.toHaveBeenCalled();
    expect(uploadSource.dispatchRegisteredEvent).not.toHaveBeenCalled();
  });

  it("makes captured ports inert before retirement callbacks and rejects invalid reasons", () => {
    const runtime = new FeatureRuntime();
    const uploads = feature("uploads");
    const asynchronous = feature("async");
    const uploadSource = islandSource("stale-upload");
    const asyncSource = islandSource("stale-async");
    runtime.register(uploads.feature);
    runtime.register(asynchronous.feature);
    runtime.start();
    runtime.connectIsland(uploadSource);
    runtime.connectIsland(asyncSource);
    const uploadPort = uploads.islandPorts[0] as UploadsRuntimeIslandPort | undefined;
    const asyncPort = asynchronous.islandPorts[1] as AsyncRuntimeIslandPort | undefined;

    expect(asyncPort?.enqueueFreshRender("invalid" as FreshRenderReason)).toBe("retired");
    runtime.retireIsland(uploadSource.element);
    runtime.retireIsland(asyncSource.element);
    expect(asyncPort?.enqueueFreshRender("stream")).toBe("retired");
    expect(uploadPort?.proposeUploadHandle("avatar", null)).toBe("retired");
    expect(() => asyncPort?.writePresentationSignal("root-scope", "progress", 1)).toThrow(
      "stale_driver_port",
    );
  });

  it("isolates one slot's document and island failures from the other", () => {
    const runtime = new FeatureRuntime();
    const uploads = feature("uploads", {
      connectDocument() {
        throw new Error("document-secret-sentinel");
      },
    });
    const asynchronous = feature("async", {
      connectDocument(context) {
        asynchronous.documentContexts.push(context);
        return {
          connectIsland() {
            throw new Error("island-secret-sentinel");
          },
          dispose: asynchronous.counters.disposeDocument,
        };
      },
    });

    runtime.register(uploads.feature);
    runtime.register(asynchronous.feature);
    runtime.start();
    expect(() => runtime.connectIsland(islandSource("failure"))).not.toThrow();
    expect(JSON.stringify(runtime.driver.diagnostics)).not.toContain("secret-sentinel");
  });
});

describe("driver lifecycle and reentrancy", () => {
  it("suspends, resumes, retires, and disposes every optional owner exactly once", () => {
    const runtime = new FeatureRuntime();
    const uploads = feature("uploads");
    const source = islandSource("lifecycle");
    const documentDisposer = vi.fn(() => {
      throw new Error("isolated-document-disposer");
    });
    const islandDisposer = vi.fn(() => {
      throw new Error("isolated-island-disposer");
    });
    runtime.register(uploads.feature);
    runtime.start();
    runtime.connectIsland(source);
    uploads.documentContexts[0]?.onDispose(documentDisposer);
    uploads.islandPorts[0]?.onDispose(islandDisposer);
    runtime.suspend();
    runtime.suspend();
    runtime.resume();
    runtime.resume();
    runtime.retireIsland(source.element);
    runtime.retireIsland(source.element);

    expect(uploads.counters.disposeIsland).toHaveBeenCalledOnce();
    expect(islandDisposer).toHaveBeenCalledOnce();
    runtime.dispose();
    runtime.dispose();
    expect(uploads.counters.suspendDocument).toHaveBeenCalledOnce();
    expect(uploads.counters.suspendIsland).toHaveBeenCalledOnce();
    expect(uploads.counters.resumeDocument).toHaveBeenCalledOnce();
    expect(uploads.counters.resumeIsland).toHaveBeenCalledOnce();
    expect(uploads.counters.disposeDocument).toHaveBeenCalledOnce();
    expect(documentDisposer).toHaveBeenCalledOnce();
  });

  it("does not resurrect a retired lifecycle through a late feature", () => {
    const runtime = new FeatureRuntime();
    runtime.start();
    runtime.dispose();

    expect(runtime.register(feature("uploads").feature)).toBe("incompatible");
    expect(runtime.connectIsland(islandSource("too-late"))).toBe("retired");
  });

  it("handles feature registration during connect and rejects it during disposal", () => {
    const runtime = new FeatureRuntime();
    const asynchronous = feature("async");
    const uploads = feature("uploads", {
      connectDocument() {
        expect(runtime.register(asynchronous.feature)).toBe("registered");
        return {
          connectIsland(port) {
            uploads.counters.connectIsland(port);
            return undefined;
          },
          dispose() {
            expect(runtime.register(feature("uploads").feature)).toBe("incompatible");
          },
        };
      },
    });

    runtime.register(uploads.feature);
    runtime.start();
    runtime.connectIsland(islandSource("reentrant"));
    expect(asynchronous.counters.connectDocument).toHaveBeenCalledOnce();
    expect(asynchronous.counters.connectIsland).toHaveBeenCalledOnce();
    expect(() => {
      runtime.dispose();
    }).not.toThrow();
  });

  it("retires a reentrant island claim before accepting a returned controller", () => {
    const runtime = new FeatureRuntime();
    const controllerDispose = vi.fn();
    const uploads = feature("uploads", {
      connectDocument() {
        return {
          connectIsland(port) {
            runtime.retireIsland(port.element);
            return { dispose: controllerDispose };
          },
          dispose: vi.fn(),
        };
      },
    });
    runtime.register(uploads.feature);
    runtime.start();
    runtime.connectIsland(islandSource("reentrant-retire"));

    expect(controllerDispose).toHaveBeenCalledOnce();
  });

  it("disposes a document controller returned after reentrant stop", () => {
    const runtime = new FeatureRuntime();
    const lateControllerDisposer = vi.fn();
    const uploads = feature("uploads", {
      connectDocument() {
        runtime.dispose();
        return { connectIsland: () => undefined, dispose: lateControllerDisposer };
      },
    });

    runtime.register(uploads.feature);
    runtime.start();
    expect(lateControllerDisposer).toHaveBeenCalledOnce();
    expect(runtime.size()).toBe(0);
  });
});

describe("shared ESM and classic registration timing", () => {
  it("connects a suspended Stimulus registration once on resume", () => {
    const trace: string[] = [];
    const runtime = new FeatureRuntime({
      application: {
        load(...definitions) {
          trace.push(`stimulus:load:${String(definitions.length)}`);
        },
        start() {
          trace.push("stimulus:start");
        },
        stop() {
          trace.push("stimulus:stop");
        },
        unload(...identifiers) {
          trace.push(`stimulus:unload:${identifiers.join(",")}`);
        },
      },
      definitions: [{ identifier: "menu" }],
    });
    const uploads = defineUploadsFeature({
      connectDocument() {
        return {
          connectIsland() {
            return {
              dispose: vi.fn(),
              resume() {
                trace.push("uploads:island:resume");
              },
              suspend: vi.fn(),
            };
          },
          dispose: vi.fn(),
          resume() {
            trace.push("uploads:document:resume");
          },
          suspend: vi.fn(),
        };
      },
    });
    runtime.register(uploads);
    runtime.start();
    runtime.connectIsland(islandSource("suspended-stimulus"));
    runtime.suspend();

    expect(installStimulusAdapter(runtime.target)).toBe("registered");
    expect(trace).toEqual([]);
    runtime.resume();
    runtime.resume();
    expect(trace).toEqual([
      "stimulus:load:1",
      "stimulus:start",
      "uploads:document:resume",
      "uploads:island:resume",
    ]);

    runtime.suspend();
    runtime.resume();
    runtime.dispose();
    runtime.dispose();
    expect(trace).toEqual([
      "stimulus:load:1",
      "stimulus:start",
      "uploads:document:resume",
      "uploads:island:resume",
      "uploads:document:resume",
      "uploads:island:resume",
      "stimulus:unload:menu",
      "stimulus:stop",
    ]);
  });

  it("loads the optional Stimulus singleton once and owns ordered morph lifecycle", () => {
    const target = Object.create(null) as typeof globalThis;
    const trace: string[] = [];
    const runtime = new DriverRuntime({
      application: {
        load(...definitions) {
          trace.push(`load:${String(definitions.length)}`);
        },
        start() {
          trace.push("start");
        },
        stop() {
          trace.push("stop");
        },
        unload(...identifiers) {
          trace.push(`unload:${identifiers.join(",")}`);
        },
      },
      definitions: [{ identifier: "menu" }],
    });
    const scope = {
      closest: () => null,
      getAttribute: () => null,
      matches: () => false,
      nodeType: 1,
      querySelectorAll: () => [],
    } as unknown as Element;

    expect(installStimulusAdapter(target)).toBe("registered");
    expect(installStimulusAdapter(target)).toBe("already_registered");
    adoptClassicFeatures(target, runtime);
    runtime.start();
    runtime.connectIsland(islandSource("stimulus", scope));
    expect(runtime.morph(6, scope)).toBe(true);
    expect(runtime.morph(7, scope)).toBe(true);
    expect(runtime.morph(8, scope)).toBe(true);
    runtime.dispose();
    runtime.dispose();

    expect(trace).toEqual(["load:1", "start", "unload:menu", "stop"]);
  });

  it("delivers morph begin, success, and abort only to the claimed feature island", () => {
    const runtime = new FeatureRuntime();
    const uploads = feature("uploads");
    const first = islandSource("feature-morph-first");
    const stranger = islandSource("feature-morph-stranger");
    runtime.register(uploads.feature);
    runtime.start();
    runtime.connectIsland(first);

    expect(runtime.driver.morph(6, first.element)).toBe(true);
    expect(runtime.driver.morph(7, first.element)).toBe(true);
    expect(runtime.driver.morph(6, first.element)).toBe(true);
    expect(runtime.driver.morph(8, first.element)).toBe(true);
    expect(runtime.driver.morph(6, stranger.element)).toBe(true);

    expect(uploads.counters.beforeMorph).toHaveBeenCalledTimes(2);
    expect(uploads.counters.afterMorph).toHaveBeenCalledOnce();
    expect(uploads.counters.abortMorph).toHaveBeenCalledOnce();

    runtime.retireIsland(first.element);
    expect(runtime.driver.morph(6, first.element)).toBe(true);
    expect(uploads.counters.beforeMorph).toHaveBeenCalledTimes(2);
    runtime.dispose();
  });

  it("keeps uploads and async independent from the optional Stimulus adapter", () => {
    const target = Object.create(null) as typeof globalThis;
    const start = vi.fn();
    const runtime = new DriverRuntime({
      application: {
        load: vi.fn(),
        start,
        stop: vi.fn(),
        unload: vi.fn(),
      },
    });
    registerRuntimeFeature(target, feature("uploads").feature);
    adoptClassicFeatures(target, runtime);
    runtime.start();

    expect(start).not.toHaveBeenCalled();
    expect(runtime.diagnostics).toEqual(["contract_mismatch"]);
  });

  it("accepts one exact Stimulus adapter identity and conflicts another", () => {
    const target = Object.create(null) as typeof globalThis;
    const bridge = Object.freeze({
      afterMorph: vi.fn(),
      beforeMorph: vi.fn(() =>
        Object.freeze({ roots: [], scope: {} as Element, scopeIdentity: null }),
      ),
      dispose: vi.fn(),
      disposeScope: vi.fn(),
    });
    const adapter = Object.freeze([
      RUNTIME_STIMULUS_ADAPTER_FORMAT,
      1,
      RUNTIME_FEATURE_CORE_RANGE,
      RUNTIME_STIMULUS_ADAPTER_IDENTITY,
      () => bridge,
    ]) as unknown as RuntimeStimulusAdapter;
    const equivalent = Object.freeze([
      RUNTIME_STIMULUS_ADAPTER_FORMAT,
      1,
      RUNTIME_FEATURE_CORE_RANGE,
      RUNTIME_STIMULUS_ADAPTER_IDENTITY,
      () => bridge,
    ]) as unknown as RuntimeStimulusAdapter;
    const conflict = Object.freeze([
      RUNTIME_STIMULUS_ADAPTER_FORMAT,
      1,
      RUNTIME_FEATURE_CORE_RANGE,
      Symbol("conflicting-stimulus-adapter"),
      () => bridge,
    ]) as unknown as RuntimeStimulusAdapter;

    expect(registerRuntimeStimulusAdapter(target, adapter)).toBe("registered");
    expect(registerRuntimeStimulusAdapter(target, equivalent)).toBe("already_registered");
    expect(registerRuntimeStimulusAdapter(target, conflict)).toBe("conflict");
  });

  it("disposes a Stimulus bridge returned after reentrant runtime retirement", () => {
    const bridgeDispose = vi.fn();
    const runtime = new FeatureRuntime({
      application: {
        load: vi.fn(),
        start: vi.fn(),
        stop: vi.fn(),
        unload: vi.fn(),
      },
    });
    const bridge = Object.freeze({
      afterMorph: vi.fn(),
      beforeMorph: vi.fn(() =>
        Object.freeze({ roots: [], scope: {} as Element, scopeIdentity: null }),
      ),
      dispose: bridgeDispose,
      disposeScope: vi.fn(),
    });
    const adapter = Object.freeze([
      RUNTIME_STIMULUS_ADAPTER_FORMAT,
      1,
      RUNTIME_FEATURE_CORE_RANGE,
      RUNTIME_STIMULUS_ADAPTER_IDENTITY,
      () => {
        runtime.dispose();
        return bridge;
      },
    ]) as unknown as RuntimeStimulusAdapter;
    runtime.start();

    expect(registerRuntimeStimulusAdapter(runtime.target, adapter)).toBe("registered");
    expect(bridgeDispose).toHaveBeenCalledOnce();
  });

  it("contains throwing Stimulus scope disposal across suspend and island retirement", () => {
    const disposeScope = vi.fn(() => {
      throw new Error("secret-scope-disposal");
    });
    const bridge = Object.freeze({
      afterMorph: vi.fn(),
      beforeMorph: vi.fn(() =>
        Object.freeze({ roots: [], scope: {} as Element, scopeIdentity: null }),
      ),
      dispose: vi.fn(),
      disposeScope,
    });
    const adapter = Object.freeze([
      RUNTIME_STIMULUS_ADAPTER_FORMAT,
      1,
      RUNTIME_FEATURE_CORE_RANGE,
      RUNTIME_STIMULUS_ADAPTER_IDENTITY,
      () => bridge,
    ]) as unknown as RuntimeStimulusAdapter;
    const runtime = new FeatureRuntime({
      application: { load: vi.fn(), start: vi.fn(), stop: vi.fn(), unload: vi.fn() },
    });
    const uploads = feature("uploads");
    runtime.register(uploads.feature);
    registerRuntimeStimulusAdapter(runtime.target, adapter);
    runtime.start();
    const island = islandSource("throwing-stimulus-scope");
    runtime.connectIsland(island);
    runtime.driver.morph(6, island.element);

    expect(() => {
      runtime.suspend();
    }).not.toThrow();
    expect(uploads.counters.suspendDocument).toHaveBeenCalledOnce();
    expect(uploads.counters.suspendIsland).toHaveBeenCalledOnce();
    runtime.resume();
    runtime.driver.morph(6, island.element);
    runtime.retireIsland(island.element);
    expect(uploads.counters.disposeIsland).toHaveBeenCalledOnce();
    expect(disposeScope).toHaveBeenCalledTimes(2);
  });

  it("contains a throwing Stimulus beforeMorph with one fixed diagnostic", () => {
    const runtime = new FeatureRuntime({
      application: { load: vi.fn(), start: vi.fn(), stop: vi.fn(), unload: vi.fn() },
    });
    registerRuntimeStimulusAdapter(
      runtime.target,
      Object.freeze([
        RUNTIME_STIMULUS_ADAPTER_FORMAT,
        1,
        RUNTIME_FEATURE_CORE_RANGE,
        RUNTIME_STIMULUS_ADAPTER_IDENTITY,
        () =>
          Object.freeze({
            afterMorph: vi.fn(),
            beforeMorph: vi.fn(() => {
              throw new Error("secret-before-morph");
            }),
            dispose: vi.fn(),
            disposeScope: vi.fn(),
          }),
      ]) as unknown as RuntimeStimulusAdapter,
    );
    runtime.start();
    const island = islandSource("throwing-stimulus-before-morph");
    runtime.connectIsland(island);

    expect(runtime.driver.morph(6, island.element)).toBe(true);
    expect(runtime.driver.diagnostics).toEqual(["operation_rejected"]);
    expect(runtime.driver.morph(7, island.element)).toBe(true);
    expect(runtime.driver.diagnostics).toEqual(["operation_rejected"]);
  });

  it("contains a throwing Stimulus document disposer and clears it before callbacks", () => {
    const dispose = vi.fn(() => {
      throw new Error("secret-stimulus-dispose");
    });
    const registry = createOptionalFeatureDriver();
    registry.registerStimulus(
      Object.freeze([
        RUNTIME_STIMULUS_ADAPTER_FORMAT,
        1,
        RUNTIME_FEATURE_CORE_RANGE,
        RUNTIME_STIMULUS_ADAPTER_IDENTITY,
        () =>
          Object.freeze({
            afterMorph: vi.fn(),
            beforeMorph: vi.fn(() =>
              Object.freeze({ roots: [], scope: {} as Element, scopeIdentity: null }),
            ),
            dispose,
            disposeScope: vi.fn(),
          }),
      ]) as unknown as RuntimeStimulusAdapter,
    );
    const driver = registry.driver;
    const documentPort: RuntimeFeatureDriverDocumentPort = Object.freeze({
      diagnose: vi.fn(),
      stimulus: {
        application: { load: vi.fn(), start: vi.fn(), stop: vi.fn(), unload: vi.fn() },
      },
    });
    expect(Reflect.apply(driver[4], driver, [0, documentPort])).toBe(true);

    expect(() => Reflect.apply(driver[4], driver, [5, null])).not.toThrow();
    expect(Reflect.apply(driver[4], driver, [5, null])).toBe(false);
    expect(dispose).toHaveBeenCalledOnce();
  });

  it("does not resurrect a Stimulus morph token after reentrant island retirement", () => {
    const disposeScope = vi.fn();
    const runtime = new FeatureRuntime({
      application: { load: vi.fn(), start: vi.fn(), stop: vi.fn(), unload: vi.fn() },
    });
    const bridge = Object.freeze({
      afterMorph: vi.fn(),
      beforeMorph: vi.fn((scope: Element) => {
        runtime.retireIsland(scope);
        return Object.freeze({ roots: [], scope, scopeIdentity: null });
      }),
      dispose: vi.fn(),
      disposeScope,
    });
    registerRuntimeStimulusAdapter(
      runtime.target,
      Object.freeze([
        RUNTIME_STIMULUS_ADAPTER_FORMAT,
        1,
        RUNTIME_FEATURE_CORE_RANGE,
        RUNTIME_STIMULUS_ADAPTER_IDENTITY,
        () => bridge,
      ]) as unknown as RuntimeStimulusAdapter,
    );
    runtime.start();
    const island = islandSource("reentrant-stimulus-retire");
    runtime.connectIsland(island);

    expect(runtime.driver.morph(6, island.element)).toBe(false);
    runtime.suspend();
    expect(disposeScope).toHaveBeenCalledOnce();
  });

  it("does not resurrect a Stimulus morph token after reentrant document stop", () => {
    const runtime = new FeatureRuntime({
      application: { load: vi.fn(), start: vi.fn(), stop: vi.fn(), unload: vi.fn() },
    });
    const bridgeDispose = vi.fn();
    registerRuntimeStimulusAdapter(
      runtime.target,
      Object.freeze([
        RUNTIME_STIMULUS_ADAPTER_FORMAT,
        1,
        RUNTIME_FEATURE_CORE_RANGE,
        RUNTIME_STIMULUS_ADAPTER_IDENTITY,
        () =>
          Object.freeze({
            afterMorph: vi.fn(),
            beforeMorph: vi.fn((scope: Element) => {
              runtime.dispose();
              return Object.freeze({ roots: [], scope, scopeIdentity: null });
            }),
            dispose: bridgeDispose,
            disposeScope: vi.fn(),
          }),
      ]) as unknown as RuntimeStimulusAdapter,
    );
    runtime.start();
    const island = islandSource("reentrant-stimulus-stop");
    runtime.connectIsland(island);

    expect(runtime.driver.morph(6, island.element)).toBe(false);
    expect(bridgeDispose).toHaveBeenCalledOnce();
  });

  it("adopts pre-boot features as one driver and keeps repeated adoption idempotent", () => {
    const target = Object.create(null) as typeof globalThis;
    const runtime = new DriverRuntime();
    const uploads = feature("uploads");

    expect(registerClassicFeature(target, uploads.feature)).toBe("registered");
    expect(registerClassicFeature(target, uploads.feature)).toBe("already_registered");
    adoptClassicFeatures(target, runtime);
    adoptClassicFeatures(target, runtime);
    runtime.start();

    expect(uploads.counters.connectDocument).toHaveBeenCalledOnce();
    expect(runtime.size()).toBe(1);
    const surface = Reflect.get(target, CLASSIC_FEATURE_SYMBOL) as object;
    expect(Object.keys(surface).sort()).toEqual(["configureAsync", "register", "version"]);
    expect("pending" in surface).toBe(false);
  });

  it("does not re-read producer accessors during driver adoption", () => {
    const target = Object.create(null) as typeof globalThis;
    const runtime = new DriverRuntime();
    const reads = vi.fn();
    const definition = new Proxy(Object.create(null) as RuntimeFeatureDefinition, {
      get(_definition, property) {
        if (property === "connectDocument") {
          reads();
          return () => ({ connectIsland: () => undefined, dispose: () => undefined });
        }
        return undefined;
      },
    });

    registerRuntimeFeature(target, defineUploadsFeature(definition));
    adoptClassicFeatures(target, runtime);
    runtime.start();
    expect(reads).toHaveBeenCalledOnce();
  });

  it("registers ESM-source features after boot through the live driver sink", () => {
    const target = Object.create(null) as typeof globalThis;
    const runtime = new DriverRuntime();
    const handle = {
      register: (driver: RuntimeFeatureDriver) => runtime.register(driver),
      status: () => "running",
      stop: () => undefined,
    };
    Object.defineProperty(target, Symbol.for("suprnova.live.runtime.v1"), { value: handle });
    runtime.start();
    const uploads = feature("uploads");
    const asynchronous = feature("async");

    expect(registerRuntimeFeature(target, uploads.feature)).toBe("registered");
    expect(registerRuntimeFeature(target, asynchronous.feature)).toBe("registered");
    expect(uploads.counters.connectDocument).toHaveBeenCalledOnce();
    expect(asynchronous.counters.connectDocument).toHaveBeenCalledOnce();
    runtime.dispose();
    expect(registerRuntimeFeature(target, uploads.feature)).toBe("incompatible");
  });

  it("closes throwing runtime-symbol and runtime-port access to one bounded outcome", () => {
    const symbol = Symbol.for("suprnova.live.runtime.v1");
    const symbolReads = vi.fn();
    const registerReads = vi.fn();
    const hostileRuntime = new Proxy<Record<PropertyKey, unknown>>(
      {},
      {
        get(_runtime, runtimeProperty) {
          if (runtimeProperty === "register") {
            registerReads();
            throw new Error("secret-register-accessor");
          }
          return () => undefined;
        },
      },
    );
    const target = new Proxy(Object.create(null) as typeof globalThis, {
      get(object, property, receiver) {
        if (property === symbol) {
          symbolReads();
          return hostileRuntime;
        }
        return Reflect.get(object, property, receiver) as unknown;
      },
    });

    expect(registerRuntimeFeature(target, feature("uploads").feature)).toBe("incompatible");
    expect(symbolReads).toHaveBeenCalledOnce();
    expect(registerReads).toHaveBeenCalledOnce();
  });

  it("snapshots the runtime symbol and register callback exactly once", () => {
    const symbol = Symbol.for("suprnova.live.runtime.v1");
    const symbolReads = vi.fn();
    const registerReads = vi.fn();
    const register = vi.fn(() => "registered" as const);
    const runtime = {
      get register(): (driver: RuntimeFeatureDriver) => RuntimeFeatureRegistrationOutcome {
        registerReads();
        if (registerReads.mock.calls.length !== 1) throw new Error("secret-register-swap");
        return register;
      },
      status: () => "running",
      stop: () => undefined,
    };
    const target = new Proxy(Object.create(null) as typeof globalThis, {
      get(object, property, receiver) {
        if (property === symbol) {
          symbolReads();
          if (symbolReads.mock.calls.length !== 1) throw new Error("secret-runtime-swap");
          return runtime;
        }
        return Reflect.get(object, property, receiver) as unknown;
      },
    });

    expect(registerRuntimeFeature(target, feature("uploads").feature)).toBe("registered");
    expect(symbolReads).toHaveBeenCalledOnce();
    expect(registerReads).toHaveBeenCalledOnce();
    expect(register).toHaveBeenCalledOnce();
    expect(register.mock.contexts[0]).toBe(runtime);
  });

  it("redacts runtime registration invocation failures", () => {
    const target = Object.create(null) as typeof globalThis;
    Object.defineProperty(target, Symbol.for("suprnova.live.runtime.v1"), {
      value: {
        register() {
          throw new Error("secret-runtime-invocation");
        },
        status: () => "running",
        stop: () => undefined,
      },
    });

    expect(registerRuntimeFeature(target, feature("uploads").feature)).toBe("incompatible");
  });

  it("isolates hostile global surfaces without exposing a runtime host to callbacks", () => {
    const target = Object.create(null) as typeof globalThis;
    Object.defineProperty(target, CLASSIC_FEATURE_SYMBOL, { value: Object.freeze({ version: 2 }) });

    expect(() => registerRuntimeFeature(target, feature("uploads").feature)).toThrow(
      "feature_global_symbol_conflict",
    );
    expect(() => {
      adoptClassicFeatures(target, new DriverRuntime());
    }).toThrow("feature_global_symbol_conflict");
  });

  it("keeps classic and ESM producers on the same bounded two-slot surface", () => {
    const target = Object.create(null) as typeof globalThis;
    const uploads = feature("uploads");
    const asynchronous = feature("async");
    const conflict = feature("uploads");

    expect(registerClassicFeature(target, uploads.feature)).toBe("registered");
    expect(registerRuntimeFeature(target, asynchronous.feature)).toBe("registered");
    expect(registerRuntimeFeature(target, conflict.feature)).toBe("conflict");
  });
});
