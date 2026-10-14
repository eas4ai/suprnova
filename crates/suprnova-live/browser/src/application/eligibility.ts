import type {
  BrowserIslandAuthority,
  ResponseEligibility,
  ResponseRequestAuthority,
  ValidatedResponse,
} from "./types.js";

export function evaluateResponseEligibility(
  response: ValidatedResponse,
  island: BrowserIslandAuthority,
  request: ResponseRequestAuthority,
): ResponseEligibility {
  if (response.correlationId !== request.correlationId) {
    return Object.freeze({ disposition: "correlation" });
  }
  if (response.protocol !== request.protocol) {
    return Object.freeze({ disposition: "protocol" });
  }
  if (request.baseRevision !== island.revision) {
    return Object.freeze({ disposition: "base_revision" });
  }
  if (request.connectionEpoch !== island.connectionEpoch) {
    return Object.freeze({ disposition: "connection_epoch" });
  }
  if (!island.active) return Object.freeze({ disposition: "retired" });
  if (request.applicationDisposition !== "accepted") {
    return Object.freeze({ disposition: "application_slot" });
  }
  if (response.kind !== "committed") return Object.freeze({ disposition: "accepted" });
  const promotion = island.snapshotForm === "seed";
  if (
    response.snapshotView.form !== "instance" ||
    promotion !== request.promotion ||
    (promotion && response.snapshotView.instanceId === null)
  ) {
    return Object.freeze({ disposition: "snapshot_form" });
  }
  if (
    response.acceptedRevision !== request.baseRevision + 1n ||
    response.snapshotView.revision !== response.acceptedRevision
  ) {
    return Object.freeze({ disposition: "successor_revision" });
  }
  if (
    response.snapshotView.component !== island.component ||
    response.snapshotView.slot !== island.slot ||
    (island.snapshotForm === "instance" && response.snapshotView.instanceId !== island.instanceId)
  ) {
    return Object.freeze({ disposition: "island" });
  }
  return Object.freeze({ disposition: "accepted" });
}
