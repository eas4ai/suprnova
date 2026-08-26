import type { JsonValue } from "../canonical.js";
import type { IslandExtensionIdentity } from "../extensions/registry.js";
import type { StimulusBootstrapOptions } from "../stimulus/port.js";
import type { UploadHandleProposal, UploadHandleProposalDisposition } from "../uploads/types.js";

export type RuntimeFeatureRegistrationOutcome =
  "registered" | "already_registered" | "incompatible" | "conflict" | "registry_full";
export type RuntimeFeatureDiagnosticDetail =
  "contract_mismatch" | "operation_rejected" | "resource_exhausted";
export type FreshRenderReason = "poll" | "stream";
export type FreshRenderDisposition = "queued" | "coalesced" | "retired";
export type RegisteredBrowserEventDisposition =
  "dispatched" | "no_target" | "fanout_exceeded" | "rejected" | "retired";

export interface RegisteredBrowserEventDispatch {
  readonly event: string;
  readonly maximumFanout: number;
  readonly payload: JsonValue;
  readonly schemaVersion: number;
  readonly target: string;
}

export interface RuntimeFeatureDriverDocumentPort {
  diagnose(detail: RuntimeFeatureDiagnosticDetail): void;
  readonly stimulus?: StimulusBootstrapOptions | undefined;
}

export interface RuntimeFeatureDriverIslandPort {
  readonly element: Element;
  readonly identity: IslandExtensionIdentity;
  dispatchRegisteredEvent(event: RegisteredBrowserEventDispatch): RegisteredBrowserEventDisposition;
  enqueueFreshRender(reason: FreshRenderReason): FreshRenderDisposition;
  proposeUploadHandle(
    field: string,
    proposal: UploadHandleProposal,
  ): UploadHandleProposalDisposition;
  writePresentationSignal(element: Element, name: string, value: JsonValue): JsonValue;
}

export type RuntimeFeatureDriverValue =
  RuntimeFeatureDriverDocumentPort | RuntimeFeatureDriverIslandPort | Element | null;
export type RuntimeFeatureDriverCallback = (
  event: 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8,
  value: RuntimeFeatureDriverValue,
) => boolean;

export const RUNTIME_FEATURE_DRIVER_FORMAT = Symbol.for("suprnova.live.feature-driver.v1");
export const RUNTIME_FEATURE_DRIVER_CORE_RANGE = 1_099_511_758_848;

export type RuntimeFeatureDriver = readonly [
  format: typeof RUNTIME_FEATURE_DRIVER_FORMAT,
  abiVersion: 1,
  packedCoreRange: typeof RUNTIME_FEATURE_DRIVER_CORE_RANGE,
  identity: object,
  drive: RuntimeFeatureDriverCallback,
];

export type InspectedRuntimeFeatureDriver = RuntimeFeatureDriver;

export interface RuntimeFeatureDriverRegistrationHost {
  register(driver: RuntimeFeatureDriver): RuntimeFeatureRegistrationOutcome;
}

export function inspectRuntimeFeatureDriver(input: unknown): InspectedRuntimeFeatureDriver | null {
  if (!Array.isArray(input) || !Object.isFrozen(input) || input.length !== 5) return null;
  try {
    const descriptors = Object.getOwnPropertyDescriptors(input);
    const identity: unknown = descriptors[3]?.value;
    if (
      Reflect.ownKeys(descriptors).length !== 6 ||
      descriptors[0]?.value !== RUNTIME_FEATURE_DRIVER_FORMAT ||
      descriptors[1]?.value !== 1 ||
      descriptors[2]?.value !== RUNTIME_FEATURE_DRIVER_CORE_RANGE ||
      (typeof identity !== "object" && typeof identity !== "function") ||
      identity === null ||
      !Object.isFrozen(identity) ||
      Reflect.ownKeys(identity).length !== 0 ||
      typeof descriptors[4]?.value !== "function"
    ) {
      return null;
    }
    return input as unknown as RuntimeFeatureDriver;
  } catch {
    return null;
  }
}
