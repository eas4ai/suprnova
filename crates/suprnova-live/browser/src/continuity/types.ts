import type { SignalContinuity } from "../signals/lifecycle.js";

export interface ContinuityLimits {
  readonly maxControls: number;
  readonly maxRetainedBytes: number;
  readonly maxScrollScopes: number;
  readonly maxSelections: number;
}

export const DEFAULT_CONTINUITY_LIMITS: ContinuityLimits = Object.freeze({
  maxControls: 64,
  maxRetainedBytes: 16_384,
  maxScrollScopes: 32,
  maxSelections: 32,
});

export type ControlContinuity =
  | {
      readonly authoritative: boolean;
      readonly checked: boolean;
      readonly element: HTMLInputElement;
      readonly identity: string;
      readonly indeterminate: boolean;
      readonly kind: "check";
    }
  | {
      readonly authoritative: boolean;
      readonly element: HTMLInputElement | HTMLTextAreaElement;
      readonly identity: string;
      readonly kind: "text";
      readonly value: string;
    }
  | {
      readonly authoritative: boolean;
      readonly element: HTMLSelectElement;
      readonly identity: string;
      readonly kind: "select";
      readonly values: readonly string[];
    }
  | {
      readonly authoritative: false;
      readonly element: HTMLInputElement;
      readonly identity: string;
      readonly kind: "file";
    };

export type SelectionRecord =
  | {
      readonly direction: "backward" | "forward" | "none";
      readonly element: HTMLInputElement | HTMLTextAreaElement;
      readonly end: number;
      readonly identity: string;
      readonly kind: "control";
      readonly start: number;
    }
  | {
      readonly endOffset: number;
      readonly endPath: readonly number[];
      readonly identity: string;
      readonly kind: "contenteditable";
      readonly root: HTMLElement;
      readonly startOffset: number;
      readonly startPath: readonly number[];
    };

export interface CompositionRecord {
  readonly data: string;
  readonly element: Element;
  readonly identity: string;
}

export interface ScrollContinuity {
  readonly element: HTMLElement;
  readonly identity: string;
  readonly left: number;
  readonly top: number;
}

export interface FocusContinuity {
  readonly element: HTMLElement | null;
  readonly focusedKey: string | null;
  readonly focusVisible: boolean;
}

export interface ContinuityRecord {
  readonly composition: CompositionRecord | null;
  readonly controls: readonly ControlContinuity[];
  readonly focusElement: HTMLElement | null;
  readonly focusedKey: string | null;
  readonly focusVisible: boolean;
  readonly scroll: readonly ScrollContinuity[];
  readonly selections: readonly SelectionRecord[];
  readonly signalScopes: readonly SignalContinuity[];
}

export class ContinuityError extends Error {
  constructor(
    readonly code:
      "incompatible_state" | "invalid_authority" | "invalid_identity" | "resource_exhausted",
  ) {
    super(`continuity_${code}`);
    this.name = "ContinuityError";
  }
}

export interface ContinuityBudget {
  bytes: number;
  readonly limit: number;
}

export function consumeContinuityBytes(budget: ContinuityBudget, value: string): void {
  budget.bytes += new TextEncoder().encode(value).byteLength;
  if (budget.bytes > budget.limit) throw new ContinuityError("resource_exhausted");
}
