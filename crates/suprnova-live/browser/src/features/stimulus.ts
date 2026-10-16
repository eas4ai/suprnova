import { createStimulusMorphBridge } from "../stimulus/bridge.js";
import {
  RUNTIME_STIMULUS_ADAPTER_FORMAT,
  RUNTIME_STIMULUS_ADAPTER_IDENTITY,
  RUNTIME_FEATURE_CORE_RANGE,
  type RuntimeStimulusAdapter,
} from "./contract.js";
import { registerRuntimeStimulusAdapter } from "./producer.js";
import type { RuntimeFeatureRegistrationOutcome } from "./host.js";

const STIMULUS_ADAPTER = Object.freeze([
  RUNTIME_STIMULUS_ADAPTER_FORMAT,
  1,
  RUNTIME_FEATURE_CORE_RANGE,
  RUNTIME_STIMULUS_ADAPTER_IDENTITY,
  createStimulusMorphBridge,
]) as unknown as RuntimeStimulusAdapter;

export function installStimulusAdapter(
  target: typeof globalThis = globalThis,
): RuntimeFeatureRegistrationOutcome {
  return registerRuntimeStimulusAdapter(target, STIMULUS_ADAPTER);
}
