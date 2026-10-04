import type { MorphPlan } from "../morph/types.js";
import type { ScrollContinuity } from "./types.js";

const SCROLL_ATTRIBUTE = "data-suprnova-live-scroll";

export function captureScroll(plan: MorphPlan): readonly ScrollContinuity[] {
  const records: ScrollContinuity[] = [];
  for (const entry of plan.identity.entries) {
    if (!(entry.current instanceof HTMLElement) || !entry.current.hasAttribute(SCROLL_ATTRIBUTE)) {
      continue;
    }
    records.push(
      Object.freeze({
        element: entry.current,
        identity: entry.token,
        left: entry.current.scrollLeft,
        top: entry.current.scrollTop,
      }),
    );
  }
  return Object.freeze(records);
}

export function restoreScroll(root: HTMLElement, records: readonly ScrollContinuity[]): void {
  for (const record of records) {
    if (!record.element.isConnected || !root.contains(record.element)) continue;
    record.element.scrollTo({ behavior: "instant", left: record.left, top: record.top });
  }
}
