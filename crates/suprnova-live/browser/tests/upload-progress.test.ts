import { describe, expect, it } from "vitest";

import { UploadProgressPresenter, createUploadProgressView } from "../src/uploads/progress.js";
import type { UploadPresentationState, UploadTransferSnapshot } from "../src/uploads/types.js";

class AttributeElement {
  readonly #attributes = new Map<string, string>();

  constructor(readonly tagName = "DIV") {}

  getAttribute(name: string): string | null {
    return this.#attributes.get(name) ?? null;
  }

  removeAttribute(name: string): void {
    this.#attributes.delete(name);
  }

  setAttribute(name: string, value: string): void {
    this.#attributes.set(name, value);
  }
}

function root(tagName = "DIV"): Element {
  return new AttributeElement(tagName) as unknown as Element;
}

function snapshot(
  state: UploadPresentationState,
  sentBytes = 25,
  size = 100,
): UploadTransferSnapshot {
  return Object.freeze({
    field: "attachment",
    handle: "018f47c1-2af0-7cc4-a001-000000000001",
    name: "report.pdf",
    retainedChunks: 0,
    revision: "2",
    sentBytes,
    size,
    state,
  });
}

const STATES: readonly UploadPresentationState[] = Object.freeze([
  "queued",
  "transferring",
  "verifying",
  "ready",
  "finalizing",
  "finalized",
  "interrupted",
  "failed",
  "canceled",
  "expired",
]);

describe("upload progress projection", () => {
  it.each(STATES)("projects the truthful %s state with bounded numeric progress", (state) => {
    const target = root();
    const presenter = new UploadProgressPresenter({ now: () => 0 });
    const view = createUploadProgressView([snapshot(state)]);

    expect(view).not.toBeNull();
    if (view === null) throw new Error("upload_progress_view_missing");
    presenter.render(target, view);

    expect(target.getAttribute("data-live-upload-state")).toBe(state);
    expect(target.getAttribute("data-live-upload-loaded")).toBe("25");
    expect(target.getAttribute("data-live-upload-total")).toBe("100");
    expect(target.getAttribute("data-live-upload-percent")).toBe("25");
    expect(target.getAttribute("aria-valuemin")).toBe("0");
    expect(target.getAttribute("aria-valuemax")).toBe("100");
    expect(target.getAttribute("aria-valuenow")).toBe("25");
  });

  it("clamps impossible byte counts and does not claim completion before verification", () => {
    expect(createUploadProgressView([snapshot("transferring", 150, 100)])).toEqual({
      loadedBytes: 100,
      percent: 100,
      state: "transferring",
      totalBytes: 100,
    });
    expect(createUploadProgressView([snapshot("transferring", -1, 100)])).toEqual({
      loadedBytes: 0,
      percent: 0,
      state: "transferring",
      totalBytes: 100,
    });
  });

  it("throttles numeric announcements while projecting current visual progress", () => {
    let now = 0;
    const target = root();
    const presenter = new UploadProgressPresenter({ announceEveryMs: 500, now: () => now });

    presenter.render(target, {
      loadedBytes: 10,
      percent: 10,
      state: "transferring",
      totalBytes: 100,
    });
    now = 100;
    presenter.render(target, {
      loadedBytes: 20,
      percent: 20,
      state: "transferring",
      totalBytes: 100,
    });

    expect(target.getAttribute("data-live-upload-percent")).toBe("20");
    expect(target.getAttribute("aria-valuenow")).toBe("10");

    now = 500;
    presenter.render(target, {
      loadedBytes: 30,
      percent: 30,
      state: "transferring",
      totalBytes: 100,
    });
    expect(target.getAttribute("aria-valuenow")).toBe("30");
  });

  it("announces state changes immediately and exposes error association", () => {
    const target = root();
    target.setAttribute("aria-errormessage", "attachment-error");
    const presenter = new UploadProgressPresenter({ announceEveryMs: 500, now: () => 0 });

    presenter.render(target, {
      loadedBytes: 25,
      percent: 25,
      state: "transferring",
      totalBytes: 100,
    });
    presenter.render(target, {
      loadedBytes: 25,
      percent: 25,
      state: "failed",
      totalBytes: 100,
    });

    expect(target.getAttribute("aria-valuetext")).toBe("Upload failed at 25%");
    expect(target.getAttribute("aria-invalid")).toBe("true");
    expect(target.getAttribute("aria-errormessage")).toBe("attachment-error");
  });

  it("marks reduced-motion presentation without changing semantic state", () => {
    const target = root("PROGRESS");
    const presenter = new UploadProgressPresenter({ now: () => 0, reducedMotion: () => true });

    presenter.render(target, {
      loadedBytes: 100,
      percent: 100,
      state: "verifying",
      totalBytes: 100,
    });

    expect(target.getAttribute("data-live-upload-motion")).toBe("reduced");
    expect(target.getAttribute("data-live-upload-state")).toBe("verifying");
    expect(target.getAttribute("aria-busy")).toBe("true");
    expect(target.getAttribute("role")).toBeNull();
  });
});
