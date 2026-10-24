import type { JsonObject, JsonValue } from "../canonical.js";
import type { SnapshotPublicView } from "../protocol/snapshot-view.js";

export type ResponseOutcome = "accepted" | "duplicate" | "rejected" | "refresh_required" | "fatal";

export type ErrorCategory =
  | "protocol"
  | "validation"
  | "authentication"
  | "authorization"
  | "csrf"
  | "snapshot"
  | "revision"
  | "render"
  | "morph"
  | "provider"
  | "cache"
  | "upload"
  | "compatibility"
  | "size_limit"
  | "rate_limit"
  | "security"
  | "internal";

export type RecoveryInstruction =
  "retain_dom" | "retry" | "refresh_island" | "remount_island" | "navigate" | "stop";

export interface ValidatedEmission {
  readonly name: string;
  readonly payload: JsonValue;
}

export interface ValidatedLiveError {
  readonly category: ErrorCategory;
  readonly detail: string;
  readonly recovery: RecoveryInstruction;
}

export interface ValidatedChildDelivery {
  readonly childInstance: string;
  readonly envelope: JsonObject;
  readonly parameterHash: string;
}

export type ValidatedRender =
  Readonly<{ kind: "html"; html: string }> | Readonly<{ kind: "no_render" }>;

interface ValidatedBaseResponse {
  readonly correlationId: string;
  readonly effects: readonly ValidatedEmission[];
  readonly events: readonly ValidatedEmission[];
  readonly extensions: JsonObject;
  readonly outcome: ResponseOutcome;
  readonly protocol: 1 | 2;
  readonly validation: JsonObject;
}

export interface ValidatedNavigationResponse extends ValidatedBaseResponse {
  readonly kind: "navigation";
  readonly navigation: "redirect" | "navigated";
  readonly target: string;
}

export interface ValidatedCommittedResponse extends ValidatedBaseResponse {
  readonly acceptedRevision: bigint;
  readonly childDeliveries: readonly ValidatedChildDelivery[];
  readonly kind: "committed";
  readonly render: ValidatedRender;
  readonly snapshot: JsonObject;
  readonly snapshotView: SnapshotPublicView;
  readonly reflectedUrl: string | null;
}

export interface ValidatedRejectedResponse extends ValidatedBaseResponse {
  readonly error: ValidatedLiveError;
  readonly kind: "rejected";
  readonly outcome: "rejected";
  readonly recovery: "retain_dom" | "retry";
}

export interface ValidatedRecoveryResponse extends ValidatedBaseResponse {
  readonly error: ValidatedLiveError;
  readonly kind: "recovery";
  readonly outcome: "refresh_required";
  readonly recovery: "refresh_island" | "remount_island" | "navigate";
}

export interface ValidatedFatalResponse extends ValidatedBaseResponse {
  readonly error: ValidatedLiveError;
  readonly kind: "fatal";
  readonly outcome: "fatal";
  readonly recovery: "stop" | "navigate";
}

export type ValidatedResponse =
  | ValidatedNavigationResponse
  | ValidatedCommittedResponse
  | ValidatedRejectedResponse
  | ValidatedRecoveryResponse
  | ValidatedFatalResponse;

export interface BrowserIslandAuthority {
  readonly active: boolean;
  readonly component: string;
  readonly connectionEpoch: number;
  readonly documentKey: string;
  readonly instanceId: string | null;
  readonly revision: bigint;
  readonly slot: string;
  readonly snapshotForm: "seed" | "instance";
}

export interface ResponseRequestAuthority {
  readonly applicationDisposition: string;
  readonly baseRevision: bigint;
  readonly connectionEpoch: number;
  readonly correlationId: string;
  readonly protocol: 1 | 2;
  readonly promotion: boolean;
}

export type EligibilityDisposition =
  | "accepted"
  | "correlation"
  | "protocol"
  | "base_revision"
  | "connection_epoch"
  | "retired"
  | "application_slot"
  | "successor_revision"
  | "island"
  | "snapshot_form";

export interface ResponseEligibility {
  readonly disposition: EligibilityDisposition;
}
