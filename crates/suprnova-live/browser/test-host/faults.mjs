/** Closed server-owned Iteration 004 fault schedule identities. */
export const iteration004FaultSchedules = Object.freeze({
  none: "none",
  sequenceGapOnce: "sequence_gap_once",
  uploadBodyInterruptedOnce: "upload_body_interrupted_once",
});

/**
 * Resolves a checked scenario name without accepting commands, paths, credentials,
 * query fragments, or arbitrary payloads.
 */
export function iteration004FaultSchedule(name) {
  if (typeof name !== "string" || !Object.hasOwn(iteration004FaultSchedules, name)) {
    throw new TypeError("unknown_iteration_004_fault_schedule");
  }
  return iteration004FaultSchedules[name];
}
