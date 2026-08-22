import { boot, createPublicApi, RUNTIME_SYMBOL } from "./bootstrap.js";
import {
  ENGINE_VERSION,
  RUNTIME_CONTRACT_VERSION,
  SUPPORTED_PROTOCOL_VERSIONS,
} from "./version.js";

export const version = ENGINE_VERSION;
export const runtimeContractVersion = RUNTIME_CONTRACT_VERSION;
export const supportedProtocolVersions = SUPPORTED_PROTOCOL_VERSIONS;
export { boot, RUNTIME_SYMBOL };
export type {
  BootstrapOptions,
  RuntimeHandle,
  RuntimeStatus,
  SuprnovaLivePublicApi,
} from "./bootstrap.js";
export type { RuntimeAsset, RuntimeAssetManifest } from "./assets.js";
export type {
  NavigationPort,
  RuntimeClock,
  RuntimeFeatures,
  RuntimeObserverFactory,
  RuntimePortOverrides,
  RuntimeRandomness,
  RuntimeScheduler,
  TransportPort,
} from "./runtime/ports.js";
export type { JsonValue } from "./canonical.js";
export type { PayloadSchema } from "./extensions/schema.js";
export type {
  EffectContext,
  EffectInvocation,
  EffectRegistration,
  EffectRunOutcome,
} from "./extensions/effects.js";
export type { RuntimeCallContext, RuntimeCallRegistration } from "./extensions/calls.js";
export type {
  StimulusApplicationPort,
  StimulusBootstrapOptions,
  StimulusContinuity,
  StimulusContinuityRoot,
  StimulusMorphBridge,
} from "./stimulus/port.js";

export default createPublicApi();
