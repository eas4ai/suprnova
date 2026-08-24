import {
  createOptionalFeatureDriver,
  type RuntimeFeature,
  type RuntimeStimulusAdapter,
} from "./contract.js";
import {
  CLASSIC_FEATURE_ADOPT_SYMBOL,
  CLASSIC_FEATURE_SYMBOL,
  type ClassicFeatureSurface,
} from "./global.js";
import type { RuntimeFeatureRegistrationOutcome } from "./host.js";

const CLASSIC_STIMULUS_ADAPTER_SYMBOL = Symbol.for("suprnova.live.features.v1.stimulus-adapter");
type ProducerFeatureSurface = ClassicFeatureSurface & {
  readonly [CLASSIC_STIMULUS_ADAPTER_SYMBOL]: (
    adapter: RuntimeStimulusAdapter,
  ) => RuntimeFeatureRegistrationOutcome;
};

type InspectedClassicSurface = readonly [
  surface: ClassicFeatureSurface,
  register: ClassicFeatureSurface["register"],
  driver: ClassicFeatureSurface[typeof CLASSIC_FEATURE_ADOPT_SYMBOL],
  stimulus: ProducerFeatureSurface[typeof CLASSIC_STIMULUS_ADAPTER_SYMBOL],
];

function inspectClassicFeatureSurface(target: typeof globalThis): InspectedClassicSurface | null {
  let current: unknown;
  try {
    current = Reflect.get(target, CLASSIC_FEATURE_SYMBOL);
  } catch {
    throw new Error("feature_global_symbol_conflict");
  }
  if (current === undefined) return null;
  let version: unknown;
  let register: unknown;
  let driver: unknown;
  let stimulus: unknown;
  try {
    version = Reflect.get(current as object, "version");
    register = Reflect.get(current as object, "register");
    driver = Reflect.get(current as object, CLASSIC_FEATURE_ADOPT_SYMBOL);
    stimulus = Reflect.get(current as object, CLASSIC_STIMULUS_ADAPTER_SYMBOL);
  } catch {
    throw new Error("feature_global_symbol_conflict");
  }
  if (
    (typeof current !== "object" && typeof current !== "function") ||
    current === null ||
    version !== 1 ||
    typeof register !== "function" ||
    typeof driver !== "function" ||
    typeof stimulus !== "function"
  ) {
    throw new Error("feature_global_symbol_conflict");
  }
  return [
    current as ClassicFeatureSurface,
    register as ClassicFeatureSurface["register"],
    driver as ClassicFeatureSurface[typeof CLASSIC_FEATURE_ADOPT_SYMBOL],
    stimulus as ProducerFeatureSurface[typeof CLASSIC_STIMULUS_ADAPTER_SYMBOL],
  ];
}

function outcome(value: unknown): RuntimeFeatureRegistrationOutcome {
  return value === "registered" ||
    value === "already_registered" ||
    value === "incompatible" ||
    value === "conflict" ||
    value === "registry_full"
    ? value
    : "incompatible";
}

function createSurface(): ClassicFeatureSurface {
  const registry = createOptionalFeatureDriver();
  const surface = {
    version: 1 as const,
    register(feature: RuntimeFeature): RuntimeFeatureRegistrationOutcome {
      return registry.register(feature);
    },
  };
  Object.defineProperty(surface, CLASSIC_FEATURE_ADOPT_SYMBOL, {
    configurable: false,
    enumerable: false,
    value: () => registry.driver,
    writable: false,
  });
  Object.defineProperty(surface, CLASSIC_STIMULUS_ADAPTER_SYMBOL, {
    configurable: false,
    enumerable: false,
    value: (adapter: RuntimeStimulusAdapter) => registry.registerStimulus(adapter),
    writable: false,
  });
  return Object.freeze(surface) as ClassicFeatureSurface;
}

function installSurface(target: typeof globalThis): InspectedClassicSurface {
  const created = createSurface();
  try {
    Object.defineProperty(target, CLASSIC_FEATURE_SYMBOL, {
      configurable: false,
      enumerable: false,
      value: created,
      writable: false,
    });
  } catch {
    throw new Error("feature_global_symbol_conflict");
  }
  const inspected = inspectClassicFeatureSurface(target);
  if (inspected === null) throw new Error("feature_global_symbol_conflict");
  return inspected;
}

function attachRunningRuntime(
  target: typeof globalThis,
  inspected: InspectedClassicSurface,
): RuntimeFeatureRegistrationOutcome | null {
  try {
    const runtime: unknown = Reflect.get(target, Symbol.for("suprnova.live.runtime.v1"));
    if (runtime === undefined) return null;
    if ((typeof runtime !== "object" && typeof runtime !== "function") || runtime === null) {
      return "incompatible";
    }
    const register: unknown = Reflect.get(runtime, "register");
    const status: unknown = Reflect.get(runtime, "status");
    const stop: unknown = Reflect.get(runtime, "stop");
    if (
      typeof register !== "function" ||
      typeof status !== "function" ||
      typeof stop !== "function"
    ) {
      return "incompatible";
    }
    const driver: unknown = Reflect.apply(inspected[2], inspected[0], []);
    return outcome(Reflect.apply(register, runtime, [driver]));
  } catch {
    return "incompatible";
  }
}

export function registerRuntimeFeature(
  target: typeof globalThis,
  feature: RuntimeFeature,
): RuntimeFeatureRegistrationOutcome {
  const inspected = inspectClassicFeatureSurface(target) ?? installSurface(target);
  const attachment = attachRunningRuntime(target, inspected);
  if (attachment !== null && attachment !== "registered" && attachment !== "already_registered") {
    return attachment;
  }
  try {
    return outcome(Reflect.apply(inspected[1], inspected[0], [feature]));
  } catch {
    return "incompatible";
  }
}

export { registerRuntimeFeature as registerClassicFeature };

export function registerRuntimeStimulusAdapter(
  target: typeof globalThis,
  adapter: RuntimeStimulusAdapter,
): RuntimeFeatureRegistrationOutcome {
  const inspected = inspectClassicFeatureSurface(target) ?? installSurface(target);
  let registration: RuntimeFeatureRegistrationOutcome;
  try {
    registration = outcome(Reflect.apply(inspected[3], inspected[0], [adapter]));
  } catch {
    return "incompatible";
  }
  if (registration !== "registered" && registration !== "already_registered") {
    return registration;
  }
  const attachment = attachRunningRuntime(target, inspected);
  return attachment === null || attachment === "registered" || attachment === "already_registered"
    ? registration
    : attachment;
}

export function installRuntimeFeatureDriver(
  target: typeof globalThis,
): RuntimeFeatureRegistrationOutcome {
  const current = inspectClassicFeatureSurface(target);
  const inspected = current ?? installSurface(target);
  return (
    attachRunningRuntime(target, inspected) ??
    (current === null ? "registered" : "already_registered")
  );
}
