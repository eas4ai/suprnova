import { describe, expect, it, vi } from "vitest";

import type {
  RuntimeFeatureDirectiveOwnership,
  RuntimeFeatureDocumentContext,
  RuntimeFeatureIslandPort,
} from "../src/features/contract.js";
import { connectUploadIsland } from "../src/uploads/feature.js";
import { UploadManager } from "../src/uploads/manager.js";
import type {
  UploadRandomness,
  UploadTransport,
  UploadTransportRequest,
  UploadTransportResponse,
} from "../src/uploads/types.js";

class SurfaceElement {
  readonly #attributes = new Map<string, string>();
  readonly addEventListener = vi.fn();
  readonly removeEventListener = vi.fn();
  disabled = false;
  files: FileList | null = null;
  multiple = false;
  textContent: string | null;
  readonly writes: Readonly<{ property: string; value: unknown }>[] = [];
  #value = "selected";

  constructor(
    readonly tagName: string,
    key: string,
    readonly type = "",
    text = "",
  ) {
    this.#attributes.set("data-suprnova-live-key", key);
    this.textContent = text;
  }

  get value(): string {
    return this.#value;
  }

  set value(value: string) {
    this.writes.push({ property: "value", value });
    this.#value = value;
  }

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

function ownership(
  name: "upload" | "progress",
  field: string,
  element: Element,
  role: "cancel" | "remove" | "retry" | null = null,
): RuntimeFeatureDirectiveOwnership {
  return {
    attributeName: `live:${name}${role === null ? "" : `.${role}`}`,
    directive: {
      capability: "uploads@1",
      modifiers: [],
      name,
      ok: true,
      role,
      value: field,
    },
    element,
  };
}

function surface(suffix: string) {
  const input = new SurfaceElement("INPUT", `attachment-input-${suffix}`, "file");
  const progress = new SurfaceElement("DIV", `attachment-progress-${suffix}`);
  const cancel = new SurfaceElement("BUTTON", `attachment-cancel-${suffix}`, "", "Cancel");
  const retry = new SurfaceElement("BUTTON", `attachment-retry-${suffix}`, "", "Retry");
  const remove = new SurfaceElement("BUTTON", `attachment-remove-${suffix}`, "", "Remove");
  return {
    cancel,
    input,
    ownerships: [
      ownership("upload", "attachment", input as unknown as Element),
      ownership("progress", "attachment", progress as unknown as Element),
      ownership("upload", "attachment", cancel as unknown as Element, "cancel"),
      ownership("upload", "attachment", retry as unknown as Element, "retry"),
      ownership("upload", "attachment", remove as unknown as Element, "remove"),
    ],
    progress,
    remove,
    retry,
  };
}

class Sequence implements UploadRandomness {
  #next = 0;

  idempotencyKey(): string {
    this.#next += 1;
    return `feature-${String(this.#next)}`;
  }
}

class ImmediateTransport implements UploadTransport {
  #revision = 0;

  send(request: UploadTransportRequest): Promise<UploadTransportResponse> {
    this.#revision += 1;
    if (request.operation === "create") {
      return Promise.resolve({
        grant: "feature-grant",
        handle: "018f47c1-2af0-7cc4-a001-000000000001",
        revision: String(this.#revision),
        state: "queued",
      });
    }
    return Promise.resolve({
      revision: String(this.#revision),
      state: request.operation === "complete" ? "ready" : "transferring",
    });
  }
}

describe("upload feature presentation and morph ownership", () => {
  it("projects progress, preserves compatible keys, and retires a replacement once", async () => {
    const manager = new UploadManager({
      chunkBytes: 1,
      connectivity: { online: () => true },
      maxActive: 1,
      maxItems: 8,
      maxQueueBytes: 1024,
      randomness: new Sequence(),
      transport: new ImmediateTransport(),
    });
    const initial = surface("stable");
    let ownerships = initial.ownerships;
    const port: RuntimeFeatureIslandPort = {
      element: { nodeType: 1 } as Element,
      enqueueFreshRender: () => "queued",
      identity: Object.freeze({
        component: "fixture.upload",
        documentKey: "document-upload-feature",
        slot: "slot-upload-feature",
      }),
      onDispose: vi.fn(),
      proposeUploadHandle: () => "accepted",
      queryDirectiveOwnership: () => ownerships,
      writePresentationSignal: (_element, _name, value) => value,
    };
    const context: RuntimeFeatureDocumentContext = {
      diagnose: vi.fn(),
      onDispose: vi.fn(),
    };
    const controller = connectUploadIsland(manager, context, port);

    await manager.select(
      {
        field: "attachment",
        input: initial.input as unknown as HTMLInputElement,
        island: port,
      },
      [new File([new Uint8Array([1])], "feature.bin")],
    );

    expect(initial.progress.getAttribute("data-live-upload-state")).toBe("ready");
    expect(initial.progress.getAttribute("aria-valuenow")).toBe("100");
    expect(initial.cancel.disabled).toBe(false);
    expect(initial.cancel.getAttribute("aria-disabled")).toBe("false");
    expect(initial.retry.disabled).toBe(true);
    expect(initial.retry.getAttribute("aria-disabled")).toBe("true");
    expect(initial.remove.disabled).toBe(false);
    expect(initial.remove.getAttribute("aria-disabled")).toBe("false");
    controller.beforeMorph?.();
    controller.afterMorph?.();
    expect(manager.activeFields(port)).toEqual(["attachment"]);
    expect(initial.input.writes).toEqual([]);

    const replacement = surface("replacement");
    controller.beforeMorph?.();
    ownerships = replacement.ownerships;
    controller.afterMorph?.();

    expect(manager.activeFields(port)).toEqual([]);
    expect(initial.input.writes).toEqual([{ property: "value", value: "" }]);
    expect(replacement.progress.getAttribute("data-live-upload-state")).toBe("canceled");
    expect(manager.inspectSecrets()).toEqual({ chunks: 0, files: 0, grants: 0 });

    controller.dispose();
    controller.dispose();
    expect(initial.input.writes).toHaveLength(1);
    manager.dispose();
  });
});
