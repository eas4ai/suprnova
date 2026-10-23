import type { SchedulerPolicy } from "./types.js";

const SAFE_POLICY_NAME = /^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$/u;
const MAX_POLICY_PARALLELISM = 32;

export const FIFO_POLICY: SchedulerPolicy = Object.freeze({ kind: "fifo" });

export interface SchedulerPolicyEffects {
  readonly duplicatePending: boolean;
  readonly replacePending: boolean;
  readonly supersedeInFlight: boolean;
  readonly abortInFlight: boolean;
  readonly parallelGroup: string | null;
  readonly parallelMaximum: number;
}

function exactKeys(
  policy: Readonly<Record<string, unknown>>,
  expected: readonly string[],
): boolean {
  const keys = Object.keys(policy).sort();
  return keys.length === expected.length && keys.every((key, index) => key === expected[index]);
}

function safeName(value: unknown): value is string {
  return typeof value === "string" && SAFE_POLICY_NAME.test(value);
}

export function normalizeSchedulerPolicy(policy: SchedulerPolicy): SchedulerPolicy {
  const input: unknown = policy;
  if (typeof input !== "object" || input === null) throw new Error("scheduler_policy_invalid");
  const candidate = input as Readonly<Record<string, unknown>>;
  switch (candidate["kind"]) {
    case "fifo":
      if (!exactKeys(candidate, ["kind"])) throw new Error("scheduler_policy_invalid");
      return FIFO_POLICY;
    case "replace_pending":
    case "drop_duplicate": {
      if (!exactKeys(candidate, ["key", "kind"]) || !safeName(candidate["key"])) {
        throw new Error("scheduler_policy_invalid");
      }
      return Object.freeze({ kind: candidate["kind"], key: candidate["key"] });
    }
    case "latest_only": {
      if (
        !exactKeys(candidate, ["abortInFlight", "key", "kind"]) ||
        !safeName(candidate["key"]) ||
        typeof candidate["abortInFlight"] !== "boolean"
      ) {
        throw new Error("scheduler_policy_invalid");
      }
      return Object.freeze({
        abortInFlight: candidate["abortInFlight"],
        key: candidate["key"],
        kind: "latest_only",
      });
    }
    case "parallel": {
      if (
        !exactKeys(candidate, ["group", "kind", "maximum"]) ||
        !safeName(candidate["group"]) ||
        !Number.isSafeInteger(candidate["maximum"]) ||
        Number(candidate["maximum"]) < 1 ||
        Number(candidate["maximum"]) > MAX_POLICY_PARALLELISM
      ) {
        throw new Error("scheduler_policy_invalid");
      }
      return Object.freeze({
        group: candidate["group"],
        kind: "parallel",
        maximum: Number(candidate["maximum"]),
      });
    }
    default:
      throw new Error("scheduler_policy_invalid");
  }
}

export function schedulerPolicyKey(policy: SchedulerPolicy): string | null {
  return "key" in policy ? policy.key : null;
}

export function schedulerPolicyEffects(policy: SchedulerPolicy): SchedulerPolicyEffects {
  switch (policy.kind) {
    case "drop_duplicate":
      return Object.freeze({
        abortInFlight: false,
        duplicatePending: true,
        parallelGroup: null,
        parallelMaximum: 1,
        replacePending: false,
        supersedeInFlight: false,
      });
    case "replace_pending":
      return Object.freeze({
        abortInFlight: false,
        duplicatePending: false,
        parallelGroup: null,
        parallelMaximum: 1,
        replacePending: true,
        supersedeInFlight: false,
      });
    case "latest_only":
      return Object.freeze({
        abortInFlight: policy.abortInFlight,
        duplicatePending: false,
        parallelGroup: null,
        parallelMaximum: 1,
        replacePending: true,
        supersedeInFlight: true,
      });
    case "parallel":
      return Object.freeze({
        abortInFlight: false,
        duplicatePending: false,
        parallelGroup: policy.group,
        parallelMaximum: policy.maximum,
        replacePending: false,
        supersedeInFlight: false,
      });
    case "fifo":
      return Object.freeze({
        abortInFlight: false,
        duplicatePending: false,
        parallelGroup: null,
        parallelMaximum: 1,
        replacePending: false,
        supersedeInFlight: false,
      });
  }
}
