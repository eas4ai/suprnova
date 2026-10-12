import type { UploadPresentationState, UploadTransferSnapshot } from "./types.js";

const ACTIVE_STATES = new Set<UploadPresentationState>([
  "queued",
  "transferring",
  "verifying",
  "finalizing",
]);
const ERROR_STATES = new Set<UploadPresentationState>(["failed", "expired"]);
const STATE_PRIORITY: readonly UploadPresentationState[] = Object.freeze([
  "failed",
  "expired",
  "interrupted",
  "transferring",
  "verifying",
  "finalizing",
  "queued",
  "ready",
  "canceled",
  "finalized",
]);

export interface UploadProgressView {
  readonly state: UploadPresentationState;
  readonly loadedBytes: number;
  readonly totalBytes: number;
  readonly percent: number | null;
}

export interface UploadProgressPresenterOptions {
  readonly announceEveryMs?: number;
  readonly now?: () => number;
  readonly reducedMotion?: () => boolean;
}

interface Announcement {
  at: number;
  state: UploadPresentationState;
}

function boundedBytes(value: number, maximum: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(Math.max(Math.trunc(value), 0), maximum);
}

function aggregateState(snapshots: readonly UploadTransferSnapshot[]): UploadPresentationState {
  for (const state of STATE_PRIORITY) {
    if (snapshots.some((snapshot) => snapshot.state === state)) return state;
  }
  return "queued";
}

function stateText(state: UploadPresentationState): string {
  switch (state) {
    case "queued":
      return "Upload queued";
    case "transferring":
      return "Uploading";
    case "verifying":
      return "Upload verifying";
    case "ready":
      return "Upload ready";
    case "finalizing":
      return "Upload finalizing";
    case "finalized":
      return "Upload finalized";
    case "interrupted":
      return "Upload interrupted";
    case "failed":
      return "Upload failed";
    case "canceled":
      return "Upload canceled";
    case "expired":
      return "Upload expired";
  }
}

function browserPrefersReducedMotion(): boolean {
  const matchMedia: unknown = Reflect.get(globalThis, "matchMedia");
  if (typeof matchMedia !== "function") return false;
  const result: unknown = Reflect.apply(matchMedia, globalThis, [
    "(prefers-reduced-motion: reduce)",
  ]) as unknown;
  if ((typeof result !== "object" && typeof result !== "function") || result === null) {
    return false;
  }
  const matches: unknown = Reflect.get(result, "matches") as unknown;
  return matches === true;
}

export function createUploadProgressView(
  snapshots: readonly UploadTransferSnapshot[],
): UploadProgressView | null {
  if (snapshots.length === 0) return null;
  let loadedBytes = 0;
  let totalBytes = 0;
  for (const snapshot of snapshots) {
    const size = boundedBytes(snapshot.size, Number.MAX_SAFE_INTEGER - totalBytes);
    totalBytes += size;
    loadedBytes += boundedBytes(snapshot.sentBytes, size);
  }
  const percent =
    totalBytes === 0
      ? snapshots.every(({ state }) => state === "ready" || state === "finalized")
        ? 100
        : 0
      : Math.min(100, Math.max(0, Math.floor((loadedBytes / totalBytes) * 100)));
  return Object.freeze({
    loadedBytes,
    percent,
    state: aggregateState(snapshots),
    totalBytes,
  });
}

export class UploadProgressPresenter {
  readonly #announceEveryMs: number;
  readonly #announcements = new WeakMap<Element, Announcement>();
  readonly #now: () => number;
  readonly #reducedMotion: () => boolean;

  constructor(options: UploadProgressPresenterOptions = {}) {
    const announceEveryMs = options.announceEveryMs ?? 500;
    if (!Number.isSafeInteger(announceEveryMs) || announceEveryMs < 0) {
      throw new RangeError("upload_progress_announcement_interval_invalid");
    }
    this.#announceEveryMs = announceEveryMs;
    this.#now = options.now ?? (() => performance.now());
    this.#reducedMotion = options.reducedMotion ?? browserPrefersReducedMotion;
  }

  render(root: Element, view: UploadProgressView): void {
    const percent = view.percent;
    root.setAttribute("data-live-upload-state", view.state);
    root.setAttribute("data-live-upload-loaded", String(view.loadedBytes));
    root.setAttribute("data-live-upload-total", String(view.totalBytes));
    if (percent === null) root.removeAttribute("data-live-upload-percent");
    else root.setAttribute("data-live-upload-percent", String(percent));
    root.setAttribute("data-live-upload-motion", this.#reducedMotion() ? "reduced" : "allowed");
    root.setAttribute("aria-busy", ACTIVE_STATES.has(view.state) ? "true" : "false");
    root.setAttribute("aria-live", "polite");
    root.setAttribute("aria-atomic", "true");
    root.setAttribute("aria-valuemin", "0");
    root.setAttribute("aria-valuemax", "100");
    if (root.tagName.toUpperCase() !== "PROGRESS") root.setAttribute("role", "progressbar");
    if (ERROR_STATES.has(view.state)) root.setAttribute("aria-invalid", "true");
    else root.removeAttribute("aria-invalid");

    const now = this.#now();
    const prior = this.#announcements.get(root);
    if (prior?.state !== view.state || now - prior.at >= this.#announceEveryMs) {
      if (percent === null) root.removeAttribute("aria-valuenow");
      else root.setAttribute("aria-valuenow", String(percent));
      root.setAttribute(
        "aria-valuetext",
        `${stateText(view.state)}${percent === null ? "" : ` at ${String(percent)}%`}`,
      );
      this.#announcements.set(root, { at: now, state: view.state });
    }
  }

  clear(root: Element): void {
    this.#announcements.delete(root);
    for (const name of [
      "aria-atomic",
      "aria-busy",
      "aria-invalid",
      "aria-live",
      "aria-valuemax",
      "aria-valuemin",
      "aria-valuenow",
      "aria-valuetext",
      "data-live-upload-loaded",
      "data-live-upload-motion",
      "data-live-upload-percent",
      "data-live-upload-state",
      "data-live-upload-total",
    ]) {
      root.removeAttribute(name);
    }
  }
}
