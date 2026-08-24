import type { JsonValue } from "../canonical.js";
import type { IslandExtensionIdentity } from "../extensions/registry.js";
import { ISLAND_ROOT_SELECTOR } from "../islands/metadata.js";
import type { RuntimeDiagnosticSink } from "../runtime/diagnostics.js";
import type {
  StimulusBootstrapOptions,
  StimulusContinuity,
  StimulusMorphBridge,
} from "../stimulus/port.js";
import type { FeatureDirectiveParseResult, ParsedFeatureDirective } from "./directive-parser.js";
import {
  RUNTIME_FEATURE_DRIVER_CORE_RANGE,
  RUNTIME_FEATURE_DRIVER_FORMAT,
  type FreshRenderDisposition,
  type FreshRenderReason,
  type RuntimeFeatureDiagnosticDetail,
  type RuntimeFeatureDriver,
  type RuntimeFeatureDriverDocumentPort,
  type RuntimeFeatureDriverIslandPort,
  type RuntimeFeatureDriverValue,
  type RuntimeFeatureRegistrationOutcome,
} from "./host.js";

export type {
  FreshRenderDisposition,
  FreshRenderReason,
  RuntimeFeatureDiagnosticDetail,
  RuntimeFeatureRegistrationOutcome,
} from "./host.js";

export type RuntimeFeatureName = "uploads" | "async";

export type RuntimeFeatureDirectiveParser = (
  attributeName: string,
  value: string,
  presentDirectiveNames?: readonly string[],
) => FeatureDirectiveParseResult;

export interface RuntimeFeatureDirectiveOwnership {
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

export interface RuntimeFeatureDefinition {
  connectDocument(context: RuntimeFeatureDocumentContext): FeatureDocumentController;
}

type RuntimeFeatureDriveValue =
  RuntimeFeatureDocumentContext | RuntimeFeatureDriverIslandPort | Element | null;
type RuntimeFeatureDrive = (
  event: 0 | 1 | 2 | 3 | 4 | 5,
  value: RuntimeFeatureDriveValue,
) => boolean;

export const RUNTIME_FEATURE_FORMAT = Symbol.for("suprnova.live.feature.v1");
export const RUNTIME_FEATURE_CORE_RANGE = RUNTIME_FEATURE_DRIVER_CORE_RANGE;
export const RUNTIME_STIMULUS_ADAPTER_FORMAT = Symbol.for(
  "suprnova.live.feature.stimulus-adapter.v1",
);
export const RUNTIME_STIMULUS_ADAPTER_IDENTITY = Symbol.for("suprnova.live.feature.stimulus.v1");

export type RuntimeFeature = readonly [
  format: typeof RUNTIME_FEATURE_FORMAT,
  slot: 0 | 1,
  capabilityVersion: 1,
  packedCoreRange: typeof RUNTIME_FEATURE_CORE_RANGE,
  identity: object,
  drive: RuntimeFeatureDrive,
];

export type RuntimeStimulusAdapter = readonly [
  format: typeof RUNTIME_STIMULUS_ADAPTER_FORMAT,
  version: 1,
  packedCoreRange: typeof RUNTIME_FEATURE_CORE_RANGE,
  identity: symbol,
  create: (
    options: StimulusBootstrapOptions,
    diagnostics: RuntimeDiagnosticSink,
  ) => StimulusMorphBridge,
];

type InspectedRuntimeFeature = readonly [
  feature: RuntimeFeature,
  slot: 0 | 1,
  drive: RuntimeFeatureDrive,
];
type NormalizedController = readonly [
  dispose: VoidFunction,
  resume: VoidFunction | null,
  suspend: VoidFunction | null,
];
type NormalizedDocumentController = readonly [
  ...NormalizedController,
  connectIsland: (port: RuntimeFeatureIslandPort) => FeatureIslandController | undefined,
];
type IslandOwnership = readonly [
  controller: NormalizedController | null,
  disposers: VoidFunction[],
];
type DriverIsland = [port: RuntimeFeatureDriverIslandPort, claims: number];

export interface OptionalFeatureDriver {
  readonly driver: RuntimeFeatureDriver;
  register(feature: RuntimeFeature): RuntimeFeatureRegistrationOutcome;
  registerStimulus(adapter: RuntimeStimulusAdapter): RuntimeFeatureRegistrationOutcome;
}

const MAXIMUM_DISPOSERS = 64;
const MAXIMUM_DRIVER_ISLANDS = 256;
const MAXIMUM_SCANNED_ELEMENTS = 4_096;
const MAXIMUM_FEATURE_DIRECTIVES = 2_048;
const UPLOADS = new WeakMap<object, RuntimeFeature>();
const ASYNC = new WeakMap<object, RuntimeFeature>();

function callback(
  owner: object,
  property:
    | "afterMorph"
    | "beforeMorph"
    | "connectDocument"
    | "connectIsland"
    | "dispose"
    | "disposeScope"
    | "resume"
    | "suspend",
  required: boolean,
): ((...arguments_: unknown[]) => unknown) | null {
  let value: unknown;
  try {
    value = Reflect.get(owner, property);
  } catch {
    throw new TypeError("feature_controller_invalid");
  }
  if (value === undefined && !required) return null;
  if (typeof value !== "function") throw new TypeError("feature_controller_invalid");
  return (...arguments_: unknown[]): unknown => Reflect.apply(value, owner, arguments_) as unknown;
}

function normalizeStimulusBridge(input: unknown): StimulusMorphBridge {
  if ((typeof input !== "object" && typeof input !== "function") || input === null) {
    throw new TypeError("feature_controller_invalid");
  }
  const dispose = callback(input, "dispose", true);
  if (dispose === null) throw new TypeError("feature_controller_invalid");
  try {
    const beforeMorph = callback(input, "beforeMorph", true);
    const afterMorph = callback(input, "afterMorph", true);
    const disposeScope = callback(input, "disposeScope", true);
    if (beforeMorph === null || afterMorph === null || disposeScope === null) {
      throw new TypeError("feature_controller_invalid");
    }
    return Object.freeze({
      afterMorph: (continuity: StimulusContinuity, scope: Element) => {
        afterMorph(continuity, scope);
      },
      beforeMorph: (scope: Element) => beforeMorph(scope) as StimulusContinuity,
      dispose: () => {
        dispose();
      },
      disposeScope: (scope: Element) => {
        disposeScope(scope);
      },
    });
  } catch (error: unknown) {
    invoke(() => {
      dispose();
    });
    throw error;
  }
}

function normalizeController(input: unknown): NormalizedController {
  if ((typeof input !== "object" && typeof input !== "function") || input === null) {
    throw new TypeError("feature_controller_invalid");
  }
  const dispose = callback(input, "dispose", true);
  if (dispose === null) throw new TypeError("feature_controller_invalid");
  try {
    const resume = callback(input, "resume", false);
    const suspend = callback(input, "suspend", false);
    return Object.freeze([
      () => {
        dispose();
      },
      resume === null
        ? null
        : () => {
            resume();
          },
      suspend === null
        ? null
        : () => {
            suspend();
          },
    ]);
  } catch (error: unknown) {
    invoke(() => {
      dispose();
    });
    throw error;
  }
}

function normalizeDocumentController(input: unknown): NormalizedDocumentController {
  const base = normalizeController(input);
  try {
    const connectIsland = callback(input as object, "connectIsland", true);
    if (connectIsland === null) throw new TypeError("feature_controller_invalid");
    return Object.freeze([
      ...base,
      (port: RuntimeFeatureIslandPort) =>
        connectIsland(port) as FeatureIslandController | undefined,
    ]);
  } catch (error: unknown) {
    invoke(base[0]);
    throw error;
  }
}

function invoke(callback: VoidFunction | null): boolean {
  try {
    callback?.();
    return true;
  } catch {
    return false;
  }
}

function own(disposers: VoidFunction[], dispose: VoidFunction): void {
  if (typeof dispose !== "function" || disposers.length >= MAXIMUM_DISPOSERS) {
    throw new TypeError("feature_disposer_invalid");
  }
  disposers.push(dispose);
}

function disposeOwnership(ownership: IslandOwnership): boolean {
  let clean = true;
  for (let index = ownership[1].length - 1; index >= 0; index -= 1) {
    clean = invoke(ownership[1][index] ?? null) && clean;
  }
  ownership[1].length = 0;
  return invoke(ownership[0]?.[0] ?? null) && clean;
}

function* featureElements(root: Element, node: Element = root): Generator<Element> {
  if (node !== root && node.matches(ISLAND_ROOT_SELECTOR)) return;
  yield node;
  for (const child of node.children) yield* featureElements(root, child);
  const shadow = "shadowRoot" in node ? node.shadowRoot : null;
  if (shadow !== null) for (const child of shadow.children) yield* featureElements(root, child);
}

function featureDirectives(
  root: Element,
  parser: RuntimeFeatureDirectiveParser,
  capability: string,
): readonly RuntimeFeatureDirectiveOwnership[] {
  if (typeof parser !== "function") return Object.freeze([]);
  const found: RuntimeFeatureDirectiveOwnership[] = [];
  let scanned = 0;
  try {
    for (const element of featureElements(root)) {
      scanned += 1;
      if (scanned > MAXIMUM_SCANNED_ELEMENTS) break;
      const attributes = [...element.attributes].filter(({ name }) => name.startsWith("live:"));
      const names = Object.freeze(attributes.map(({ name }) => name));
      for (const attribute of attributes) {
        if (found.length >= MAXIMUM_FEATURE_DIRECTIVES) return Object.freeze(found);
        const directive = parser(attribute.name, attribute.value, names);
        if (directive.ok && directive.capability === capability) {
          found.push(Object.freeze({ attributeName: attribute.name, directive, element }));
        }
      }
    }
  } catch {
    // Hostile parser and DOM access remain isolated to the optional driver.
  }
  return Object.freeze(found);
}

function defineFeature(
  slot: 0 | 1,
  definition: unknown,
  cache: WeakMap<object, RuntimeFeature>,
): RuntimeFeature {
  if ((typeof definition !== "object" && typeof definition !== "function") || definition === null) {
    throw new TypeError("feature_definition_invalid");
  }
  const cached = cache.get(definition);
  if (cached !== undefined) return cached;
  const connectDocument = callback(definition, "connectDocument", true);
  if (connectDocument === null) throw new TypeError("feature_definition_invalid");
  const identity = Object.freeze({});
  let document: NormalizedDocumentController | null = null;
  const documentDisposers: VoidFunction[] = [];
  const islands = new Map<Element, IslandOwnership>();
  let retired = false;
  let connected = false;
  const isRetired = (): boolean => retired;

  const drive: RuntimeFeatureDrive = (event, value) => {
    if (event === 0) {
      if (retired || connected || value === null || !("diagnose" in value)) return false;
      connected = true;
      const port = value;
      const context: RuntimeFeatureDocumentContext = Object.freeze({
        diagnose: (detail: RuntimeFeatureDiagnosticDetail) => {
          port.diagnose(detail);
        },
        onDispose: (dispose: VoidFunction) => {
          own(documentDisposers, dispose);
        },
      });
      let connectedDocument: NormalizedDocumentController;
      try {
        connectedDocument = normalizeDocumentController(connectDocument(context));
      } catch (error: unknown) {
        disposeOwnership([null, documentDisposers]);
        throw error;
      }
      if (isRetired()) {
        for (let index = documentDisposers.length - 1; index >= 0; index -= 1) {
          invoke(documentDisposers[index] ?? null);
        }
        documentDisposers.length = 0;
        invoke(connectedDocument[0]);
        return false;
      }
      document = connectedDocument;
      return true;
    }
    if (event === 1) {
      if (retired || document === null || value === null || !("element" in value)) return false;
      const port = value;
      const ownsIsland = (): boolean => islands.has(port.element);
      if (islands.has(port.element)) return true;
      const disposers: VoidFunction[] = [];
      let controller: NormalizedController | null = null;
      const pending: IslandOwnership = [null, disposers];
      islands.set(port.element, pending);
      const featurePort: RuntimeFeatureIslandPort = Object.freeze({
        element: port.element,
        enqueueFreshRender: (reason: FreshRenderReason) => port.enqueueFreshRender(reason),
        identity: port.identity,
        onDispose: (dispose: VoidFunction) => {
          own(disposers, dispose);
        },
        queryDirectiveOwnership: (parser: RuntimeFeatureDirectiveParser) =>
          featureDirectives(port.element, parser, slot === 0 ? "uploads@1" : "async@1"),
        writePresentationSignal: (element: Element, name: string, signalValue: JsonValue) =>
          port.writePresentationSignal(element, name, signalValue),
      });
      try {
        const connectedIsland = document[3](featurePort);
        if (connectedIsland !== undefined) controller = normalizeController(connectedIsland);
      } catch (error: unknown) {
        if (islands.get(port.element) === pending) islands.delete(port.element);
        disposeOwnership([controller, disposers]);
        throw error;
      }
      if (isRetired() || !ownsIsland()) {
        disposeOwnership([controller, disposers]);
        return false;
      }
      islands.set(port.element, [controller, disposers]);
      return true;
    }
    if (event === 4) {
      if (value === null || !("nodeType" in value)) return false;
      const ownership = islands.get(value);
      if (ownership === undefined) return true;
      islands.delete(value);
      return disposeOwnership(ownership);
    }
    if (event === 2 || event === 3) {
      let clean = true;
      const controllers = [...islands.values()];
      if (event === 2) {
        for (let index = controllers.length - 1; index >= 0; index -= 1) {
          clean = invoke(controllers[index]?.[0]?.[2] ?? null) && clean;
        }
        clean = invoke(document?.[2] ?? null) && clean;
      } else {
        clean = invoke(document?.[1] ?? null) && clean;
        for (const ownership of controllers) clean = invoke(ownership[0]?.[1] ?? null) && clean;
      }
      return clean;
    }
    if (retired) return false;
    retired = true;
    let clean = true;
    const ownerships = [...islands.values()];
    islands.clear();
    for (let index = ownerships.length - 1; index >= 0; index -= 1) {
      const ownership = ownerships[index];
      if (ownership !== undefined) clean = disposeOwnership(ownership) && clean;
    }
    for (let index = documentDisposers.length - 1; index >= 0; index -= 1) {
      clean = invoke(documentDisposers[index] ?? null) && clean;
    }
    documentDisposers.length = 0;
    return invoke(document?.[0] ?? null) && clean;
  };

  const feature = Object.freeze([
    RUNTIME_FEATURE_FORMAT,
    slot,
    1,
    RUNTIME_FEATURE_CORE_RANGE,
    identity,
    drive,
  ]) as unknown as RuntimeFeature;
  cache.set(definition, feature);
  return feature;
}

export function defineUploadsFeature(definition: RuntimeFeatureDefinition): RuntimeFeature {
  return defineFeature(0, definition, UPLOADS);
}

export function defineAsyncFeature(definition: RuntimeFeatureDefinition): RuntimeFeature {
  return defineFeature(1, definition, ASYNC);
}

function inspectRuntimeFeature(input: unknown): InspectedRuntimeFeature | null {
  if (!Array.isArray(input) || !Object.isFrozen(input) || input.length !== 6) return null;
  try {
    if (Reflect.ownKeys(input).length !== 7) return null;
    const values: unknown[] = [];
    for (let index = 0; index < 6; index += 1) {
      const descriptor = Object.getOwnPropertyDescriptor(input, index);
      if (descriptor === undefined || !("value" in descriptor)) return null;
      values.push(descriptor.value);
    }
    const slot = values[1];
    const identity = values[4];
    if (
      values[0] !== RUNTIME_FEATURE_FORMAT ||
      (slot !== 0 && slot !== 1) ||
      values[2] !== 1 ||
      values[3] !== RUNTIME_FEATURE_CORE_RANGE ||
      (typeof identity !== "object" && typeof identity !== "function") ||
      identity === null ||
      !Object.isFrozen(identity) ||
      Reflect.ownKeys(identity).length !== 0 ||
      typeof values[5] !== "function"
    ) {
      return null;
    }
    return [input as unknown as RuntimeFeature, slot, values[5] as RuntimeFeatureDrive];
  } catch {
    return null;
  }
}

function inspectStimulusAdapter(input: unknown): RuntimeStimulusAdapter | null {
  if (!Array.isArray(input) || !Object.isFrozen(input) || input.length !== 5) return null;
  try {
    if (Reflect.ownKeys(input).length !== 6) return null;
    const descriptors = Object.getOwnPropertyDescriptors(input);
    const values: unknown[] = [];
    for (let index = 0; index < 5; index += 1) values.push(descriptors[index]?.value);
    if (
      values[0] !== RUNTIME_STIMULUS_ADAPTER_FORMAT ||
      values[1] !== 1 ||
      values[2] !== RUNTIME_FEATURE_CORE_RANGE ||
      typeof values[3] !== "symbol" ||
      typeof values[4] !== "function"
    ) {
      return null;
    }
    return input as unknown as RuntimeStimulusAdapter;
  } catch {
    return null;
  }
}

export function createOptionalFeatureDriver(): OptionalFeatureDriver {
  const entries: [InspectedRuntimeFeature | null, InspectedRuntimeFeature | null] = [null, null];
  const islands = new Map<Element, DriverIsland>();
  const stimulusContinuities = new Map<Element, StimulusContinuity>();
  let documentPort: RuntimeFeatureDriverDocumentPort | null = null;
  let ready = 0;
  let size = 0;
  let started = 0;
  let state: 0 | 1 | 2 | 3 = 0;
  let stimulusAdapter: RuntimeStimulusAdapter | null = null;
  let stimulus: StimulusMorphBridge | null = null;

  const report = (detail: RuntimeFeatureDiagnosticDetail): void => {
    try {
      documentPort?.diagnose(detail);
    } catch {
      // Core diagnostics are best-effort and fixed-vocabulary.
    }
  };
  const connectStimulus = (): boolean => {
    const options = documentPort?.stimulus;
    if (stimulus !== null || options === undefined) return true;
    const adapter = stimulusAdapter;
    if (adapter === null) {
      report("contract_mismatch");
      return false;
    }
    const diagnostics: RuntimeDiagnosticSink = {
      record(input) {
        report(
          input.detailCode === "resource_exhausted" ? "resource_exhausted" : "operation_rejected",
        );
      },
    };
    try {
      const connected = normalizeStimulusBridge(
        Reflect.apply(adapter[4], adapter, [options, diagnostics]),
      );
      if (state !== 1 || stimulusAdapter !== adapter || documentPort?.stimulus !== options) {
        connected.dispose();
        return false;
      }
      stimulus = connected;
      return true;
    } catch {
      report("operation_rejected");
      return false;
    }
  };
  const run = (
    entry: InspectedRuntimeFeature,
    event: 0 | 1 | 2 | 3 | 4 | 5,
    value: RuntimeFeatureDriveValue,
  ): boolean => {
    try {
      const completed: unknown = Reflect.apply(entry[2], entry[0], [event, value]);
      if (completed === true) return true;
    } catch {
      // One feature slot cannot disable the other.
    }
    report("operation_rejected");
    return false;
  };
  const connect = (entry: InspectedRuntimeFeature, island: DriverIsland): void => {
    const bit = 1 << entry[1];
    if (state !== 1 || (ready & bit) === 0 || (island[1] & bit) !== 0) return;
    island[1] |= bit;
    run(entry, 1, island[0]);
  };
  const start = (entry: InspectedRuntimeFeature): void => {
    const bit = 1 << entry[1];
    if (state !== 1 || (started & bit) !== 0 || documentPort === null) return;
    started |= bit;
    const context: RuntimeFeatureDocumentContext = Object.freeze({
      diagnose: report,
      onDispose(dispose: VoidFunction) {
        if (typeof dispose !== "function") report("operation_rejected");
      },
    });
    if (!run(entry, 0, context) || entries[entry[1]] !== entry) return;
    ready |= bit;
    for (const island of islands.values()) connect(entry, island);
  };

  const drive = (
    event: 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8,
    value: RuntimeFeatureDriverValue,
  ): boolean => {
    if (event === 0) {
      if (state !== 0 || value === null || !("diagnose" in value)) return false;
      state = 1;
      documentPort = value;
      connectStimulus();
      for (const entry of [...entries]) if (entry !== null) start(entry);
      return true;
    }
    if (event === 1) {
      if (state !== 1 || value === null || !("element" in value)) return false;
      if (islands.has(value.element)) return true;
      if (islands.size >= MAXIMUM_DRIVER_ISLANDS) {
        report("resource_exhausted");
        return false;
      }
      const island: DriverIsland = [value, 0];
      islands.set(value.element, island);
      for (const entry of [...entries]) if (entry !== null) connect(entry, island);
      return true;
    }
    if (event === 6 || event === 7 || event === 8) {
      if (state === 3 || value === null || !("nodeType" in value)) return false;
      const bridge = stimulus;
      if (bridge === null) return true;
      if (event === 6) {
        const previous = stimulusContinuities.get(value);
        stimulusContinuities.delete(value);
        if (previous !== undefined) {
          try {
            bridge.disposeScope(value);
          } catch {
            report("operation_rejected");
          }
        }
        let continuity: StimulusContinuity;
        try {
          continuity = bridge.beforeMorph(value);
        } catch {
          report("operation_rejected");
          return true;
        }
        if (state !== 1 || stimulus !== bridge || !islands.has(value)) return false;
        stimulusContinuities.set(value, continuity);
        return true;
      }
      const continuity = stimulusContinuities.get(value);
      stimulusContinuities.delete(value);
      if (continuity !== undefined) {
        try {
          if (event === 7) bridge.afterMorph(continuity, value);
          else bridge.disposeScope(value);
        } catch {
          report("operation_rejected");
        }
      }
      return true;
    }
    if (event === 4) {
      if (state === 3 || value === null || !("nodeType" in value)) return false;
      stimulusContinuities.delete(value);
      const island = islands.get(value);
      if (island === undefined) return true;
      islands.delete(value);
      try {
        stimulus?.disposeScope(value);
      } catch {
        report("operation_rejected");
      }
      for (let slot = 1; slot >= 0; slot -= 1) {
        const entry = entries[slot];
        if (entry !== undefined && entry !== null && (island[1] & (1 << slot)) !== 0) {
          run(entry, 4, value);
        }
      }
      return true;
    }
    if (event === 2) {
      if (state !== 1) return false;
      state = 2;
      const scopes = [...stimulusContinuities.keys()];
      stimulusContinuities.clear();
      for (const scope of scopes) {
        try {
          stimulus?.disposeScope(scope);
        } catch {
          report("operation_rejected");
        }
      }
      for (let slot = 1; slot >= 0; slot -= 1) {
        const entry = entries[slot];
        if (entry !== undefined && entry !== null && (ready & (1 << slot)) !== 0) {
          run(entry, 2, null);
        }
      }
      return true;
    }
    if (event === 3) {
      if (state !== 2) return false;
      state = 1;
      for (const entry of [...entries]) {
        if (entry === null) continue;
        if ((started & (1 << entry[1])) === 0) start(entry);
        else if ((ready & (1 << entry[1])) !== 0) run(entry, 3, null);
      }
      return true;
    }
    if (state === 3) return false;
    state = 3;
    const owned = [...entries];
    const claimed = started;
    const bridge = stimulus;
    const diagnosticPort = documentPort;
    entries[0] = null;
    entries[1] = null;
    islands.clear();
    stimulusContinuities.clear();
    stimulus = null;
    stimulusAdapter = null;
    documentPort = null;
    ready = 0;
    size = 0;
    started = 0;
    for (let slot = 1; slot >= 0; slot -= 1) {
      const entry = owned[slot];
      if (entry !== undefined && entry !== null && (claimed & (1 << slot)) !== 0) {
        run(entry, 5, null);
      }
    }
    try {
      bridge?.dispose();
    } catch {
      try {
        diagnosticPort?.diagnose("operation_rejected");
      } catch {
        // Disposal diagnostics are fixed and best-effort after the live port is retired.
      }
    }
    return true;
  };

  const driver = Object.freeze([
    RUNTIME_FEATURE_DRIVER_FORMAT,
    1,
    RUNTIME_FEATURE_DRIVER_CORE_RANGE,
    Object.freeze({}),
    drive,
  ]) as unknown as RuntimeFeatureDriver;

  return Object.freeze({
    driver,
    register(feature: RuntimeFeature): RuntimeFeatureRegistrationOutcome {
      if (state === 3) return "incompatible";
      const entry = inspectRuntimeFeature(feature);
      if (entry === null) return "incompatible";
      const current = entries[entry[1]];
      if (current !== null) return current[0] === feature ? "already_registered" : "conflict";
      if (size >= 2) return "registry_full";
      entries[entry[1]] = entry;
      size += 1;
      if (state === 1) start(entry);
      return "registered";
    },
    registerStimulus(adapter: RuntimeStimulusAdapter): RuntimeFeatureRegistrationOutcome {
      if (state === 3) return "incompatible";
      const inspected = inspectStimulusAdapter(adapter);
      if (inspected === null) return "incompatible";
      if (stimulusAdapter !== null) {
        return stimulusAdapter[3] === adapter[3] ? "already_registered" : "conflict";
      }
      stimulusAdapter = inspected;
      if (state === 1) connectStimulus();
      return "registered";
    },
  });
}
