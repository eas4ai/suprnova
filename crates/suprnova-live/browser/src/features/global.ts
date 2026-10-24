import type {
  RuntimeFeatureDriver,
  RuntimeFeatureDriverRegistrationHost,
  RuntimeFeatureRegistrationOutcome,
} from "./host.js";
import type { AsyncFeatureOptions } from "../async-updates/feature.js";

export const CLASSIC_FEATURE_SYMBOL = Symbol.for("suprnova.live.features.v1");
export const CLASSIC_FEATURE_ADOPT_SYMBOL = Symbol.for("suprnova.live.features.v1.adopt");

export interface ClassicFeatureSurface {
  readonly version: 1;
  configureAsync(options: AsyncFeatureOptions): void;
  register(feature: unknown): RuntimeFeatureRegistrationOutcome;
  readonly [CLASSIC_FEATURE_ADOPT_SYMBOL]: () => RuntimeFeatureDriver;
}

export function adoptClassicFeatures(
  target: typeof globalThis,
  host: RuntimeFeatureDriverRegistrationHost,
): void {
  try {
    const current: unknown = Reflect.get(target, CLASSIC_FEATURE_SYMBOL);
    if (current === undefined) return;
    if ((typeof current !== "object" && typeof current !== "function") || current === null) {
      throw new Error();
    }
    const version: unknown = Reflect.get(current, "version");
    const register: unknown = Reflect.get(current, "register");
    const driver: unknown = Reflect.get(current, CLASSIC_FEATURE_ADOPT_SYMBOL);
    if (version !== 1 || typeof register !== "function" || typeof driver !== "function") {
      throw new Error();
    }
    const attachment: unknown = Reflect.apply(driver, current, []);
    host.register(attachment as RuntimeFeatureDriver);
  } catch {
    throw new Error("feature_global_symbol_conflict");
  }
}
