import type { SignalContinuity } from "../signals/lifecycle.js";
import type { StimulusContinuity, StimulusMorphBridge } from "../stimulus/port.js";
import { restoreFocus as restoreCapturedFocus, restoreSelections } from "./focus.js";
import { restoreControls } from "./forms.js";
import { restoreScroll } from "./scroll.js";
import { ContinuityError, type ContinuityRecord } from "./types.js";

export interface ContinuityRestorePorts {
  restoreSignals(continuity: readonly SignalContinuity[]): number;
  readonly stimulus: StimulusMorphBridge | null;
}

export function restoreContinuity(
  record: ContinuityRecord,
  root: HTMLElement,
  ports: ContinuityRestorePorts,
): void {
  restoreControls(root, record.controls);
  restoreSelections(root, record.selections);
  if (
    record.composition !== null &&
    (!record.composition.element.isConnected || !root.contains(record.composition.element))
  ) {
    throw new ContinuityError("incompatible_state");
  }
  restoreScroll(root, record.scroll);
  ports.restoreSignals(record.signalScopes);
  if (record.stimulus !== null) ports.stimulus?.afterMorph(record.stimulus, root);
}

export function restoreContinuityFocus(record: ContinuityRecord, root: HTMLElement): void {
  restoreCapturedFocus(
    root,
    Object.freeze({
      element: record.focusElement,
      focusedKey: record.focusedKey,
      focusVisible: record.focusVisible,
    }),
  );
}

export function disposeStimulusContinuity(
  continuity: StimulusContinuity | null,
  stimulus: StimulusMorphBridge | null,
  root: Element,
): void {
  if (continuity !== null) stimulus?.disposeScope(root);
}
