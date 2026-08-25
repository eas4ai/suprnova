import { describe, expect, it, vi } from "vitest";

import { UploadManager } from "../src/uploads/manager.js";
import type {
  UploadApplicationPort,
  UploadConnectivity,
  UploadIslandPort,
  UploadRandomness,
  UploadTransport,
  UploadTransportRequest,
  UploadTransportResponse,
} from "../src/uploads/types.js";

const KIB = 1024;
const HANDLE_PREFIX = "018f47c1-2af0-7cc4-a001-";

function file(name: string, size: number, type = "application/octet-stream"): File {
  return new File([new Uint8Array(size)], name, { lastModified: 1_700_000_000_000, type });
}

function input(multiple = false): HTMLInputElement {
  return { multiple, type: "file", value: "selected" } as HTMLInputElement;
}

function island(name: string) {
  const proposals: unknown[] = [];
  const port: UploadIslandPort = {
    element: { nodeType: 1 } as Element,
    identity: Object.freeze({
      component: "fixture.upload",
      documentKey: `document-${name}`,
      slot: `slot-${name}`,
    }),
    proposeUploadHandle(_field, value) {
      proposals.push(value);
      return "accepted";
    },
  };
  return { port, proposals };
}

class Online implements UploadConnectivity {
  online(): boolean {
    return true;
  }
}

class Sequence implements UploadRandomness {
  #next = 0;

  idempotencyKey(): string {
    this.#next += 1;
    return `request-${String(this.#next)}`;
  }
}

class MemoryTransport implements UploadTransport {
  readonly requests: UploadTransportRequest[] = [];
  readonly active = new Set<Promise<void>>();
  maximumActive = 0;
  #next = 0;
  readonly #revisions = new Map<string, bigint>();

  async send(request: UploadTransportRequest): Promise<UploadTransportResponse> {
    this.requests.push(request);
    let release!: () => void;
    const active = new Promise<void>((resolve) => {
      release = resolve;
    });
    this.active.add(active);
    this.maximumActive = Math.max(this.maximumActive, this.active.size);
    await Promise.resolve();
    try {
      if (request.operation === "create") {
        this.#next += 1;
        const handle = `${HANDLE_PREFIX}${this.#next.toString(16).padStart(12, "0")}`;
        this.#revisions.set(handle, 1n);
        return {
          grant: `secret-grant-${String(this.#next)}`,
          handle,
          revision: "1",
          state: "queued",
        };
      }
      const revision = (this.#revisions.get(request.handle) ?? 0n) + 1n;
      this.#revisions.set(request.handle, revision);
      return {
        ...(request.operation === "status" ? { nextChunkIndex: 2 } : {}),
        revision: revision.toString(),
        state:
          request.operation === "complete"
            ? "ready"
            : request.operation === "cancel"
              ? "canceled"
              : "transferring",
      };
    } finally {
      this.active.delete(active);
      release();
    }
  }
}

function manager(transport = new MemoryTransport(), maxActive = 4) {
  return {
    manager: new UploadManager({
      chunkBytes: 256 * KIB,
      connectivity: new Online(),
      maxActive,
      maxItems: 64,
      maxQueueBytes: 256 * KIB,
      randomness: new Sequence(),
      transport,
    }),
    transport,
  };
}

describe("current-document upload manager", () => {
  it("rejects upload settings above the upload-specific resource ceilings", () => {
    const base = {
      chunkBytes: 256 * KIB,
      connectivity: new Online(),
      maxActive: 4,
      maxItems: 64,
      maxQueueBytes: 256 * KIB,
      randomness: new Sequence(),
      transport: new MemoryTransport(),
    };
    expect(() => new UploadManager({ ...base, maxItems: 65 })).toThrow(
      "upload_manager_limits_invalid",
    );
    expect(() => new UploadManager({ ...base, maxActive: 17 })).toThrow(
      "upload_manager_limits_invalid",
    );
    expect(() => new UploadManager({ ...base, chunkBytes: 4 * 1024 * KIB + 1 })).toThrow(
      "upload_manager_limits_invalid",
    );
  });

  it("adopts an explicitly reacquired transfer and resumes real network work", async () => {
    const transport = new MemoryTransport();
    const selected = file("resume.bin", 16);
    const owner = island("resume");
    const handle = `${HANDLE_PREFIX}000000000001`;
    const reacquire = vi.fn(() =>
      Promise.resolve({
        fileIdentity: {
          lastModified: selected.lastModified,
          name: selected.name,
          size: selected.size,
          type: selected.type,
        },
        grant: "replacement-grant",
        revision: "1",
        state: "transferring" as const,
        nextChunkIndex: 2,
        uploadedBytes: 8,
      }),
    );
    const application: UploadApplicationPort = { reacquire };
    const fixture = new UploadManager({
      application,
      chunkBytes: 8,
      connectivity: new Online(),
      maxActive: 1,
      maxItems: 64,
      maxQueueBytes: 256 * KIB,
      randomness: new Sequence(),
      transport,
    });

    await fixture.reacquire(
      { field: "attachment", input: input(), island: owner.port },
      selected,
      handle,
    );

    expect(reacquire).toHaveBeenCalledOnce();
    expect(fixture.snapshot().uploads).toEqual([
      expect.objectContaining({ handle, sentBytes: 16, state: "ready" }),
    ]);
    expect(transport.requests[0]?.operation).toBe("status");
    expect(transport.requests.some(({ operation }) => operation === "create")).toBe(false);
    expect(
      transport.requests.some(
        (request) => request.operation === "put_chunk" && request.chunkIndex === 2,
      ),
    ).toBe(true);
    expect(owner.proposals[owner.proposals.length - 1]).toBe(handle);
    fixture.dispose();
  });

  it("discards a late reacquisition grant after a newer selection", async () => {
    const transport = new MemoryTransport();
    const owner = island("reacquire-race");
    const oldFile = file("old.bin", 8);
    let release!: (value: Awaited<ReturnType<UploadApplicationPort["reacquire"]>>) => void;
    const application: UploadApplicationPort = {
      reacquire: () =>
        new Promise((resolve) => {
          release = resolve;
        }),
    };
    const fixture = new UploadManager({
      application,
      chunkBytes: 4,
      connectivity: new Online(),
      maxActive: 1,
      maxItems: 64,
      maxQueueBytes: 256 * KIB,
      randomness: new Sequence(),
      transport,
    });
    const staleHandle = `${HANDLE_PREFIX}000000000099`;
    const pending = fixture.reacquire(
      { field: "attachment", input: input(), island: owner.port },
      oldFile,
      staleHandle,
    );
    await Promise.resolve();
    await fixture.select({ field: "attachment", input: input(), island: owner.port }, [
      file("new.bin", 1),
    ]);
    release({
      fileIdentity: {
        lastModified: oldFile.lastModified,
        name: oldFile.name,
        size: oldFile.size,
        type: oldFile.type,
      },
      grant: "stale-grant",
      nextChunkIndex: 1,
      revision: "1",
      state: "transferring",
      uploadedBytes: 4,
    });
    await pending;

    expect(fixture.snapshot().uploads).toEqual([
      expect.objectContaining({ name: "new.bin", state: "ready" }),
    ]);
    expect(owner.proposals[owner.proposals.length - 1]).not.toBe(staleHandle);
    expect(JSON.stringify(fixture.snapshot())).not.toContain("stale-grant");
    fixture.dispose();
  });

  it("cannot revive an island retired while reacquisition is pending", async () => {
    const owner = island("retired-reacquire");
    const selected = file("old.bin", 8);
    let release!: (value: Awaited<ReturnType<UploadApplicationPort["reacquire"]>>) => void;
    const fixture = new UploadManager({
      application: {
        reacquire: () =>
          new Promise((resolve) => {
            release = resolve;
          }),
      },
      chunkBytes: 4,
      connectivity: new Online(),
      maxActive: 1,
      maxItems: 64,
      maxQueueBytes: 256 * KIB,
      randomness: new Sequence(),
      transport: new MemoryTransport(),
    });
    const pending = fixture.reacquire(
      { field: "attachment", input: input(), island: owner.port },
      selected,
      `${HANDLE_PREFIX}000000000099`,
    );
    await Promise.resolve();
    fixture.retireIsland(owner.port);
    release({
      fileIdentity: {
        lastModified: selected.lastModified,
        name: selected.name,
        size: selected.size,
        type: selected.type,
      },
      grant: "stale-grant",
      nextChunkIndex: 1,
      revision: "1",
      state: "transferring",
      uploadedBytes: 4,
    });
    await pending;

    expect(fixture.snapshot().uploads).toEqual([]);
    expect(fixture.inspectSecrets()).toEqual({ chunks: 0, files: 0, grants: 0 });
    fixture.dispose();
  });

  it("never consults ambient persistence during selection, lifecycle, or retirement", async () => {
    const stores = ["localStorage", "sessionStorage", "indexedDB"] as const;
    const reads = vi.fn();
    const descriptors = stores.map(
      (name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)] as const,
    );
    for (const name of stores) {
      Object.defineProperty(globalThis, name, { configurable: true, get: reads });
    }
    try {
      const fixture = manager();
      const owner = island("no-persistence");
      await fixture.manager.select({ field: "attachment", input: input(), island: owner.port }, [
        file("ephemeral.bin", 3),
      ]);
      fixture.manager.suspend();
      fixture.manager.resume();
      await fixture.manager.cancel(owner.port, "attachment");
      fixture.manager.dispose();
      expect(reads).not.toHaveBeenCalled();
    } finally {
      for (const [name, descriptor] of descriptors) {
        if (descriptor === undefined) Reflect.deleteProperty(globalThis, name);
        else Object.defineProperty(globalThis, name, descriptor);
      }
    }
  });
  it("supports single, multiple, multiple fields, replacement, and repeated selection", async () => {
    const fixture = manager();
    const avatar = island("avatar");
    const documents = island("documents");
    const avatarInput = input();
    const documentsInput = input(true);

    await fixture.manager.select({ field: "avatar", input: avatarInput, island: avatar.port }, [
      file("avatar.png", 1),
    ]);
    await fixture.manager.select(
      { field: "documents", input: documentsInput, island: documents.port },
      [file("one.txt", 1), file("two.txt", 1)],
    );
    await fixture.manager.select({ field: "avatar", input: avatarInput, island: avatar.port }, [
      file("replacement.png", 1),
    ]);
    await fixture.manager.select({ field: "avatar", input: avatarInput, island: avatar.port }, [
      file("replacement.png", 1),
    ]);

    expect(fixture.manager.snapshot().uploads.map(({ field, name }) => [field, name])).toEqual([
      ["documents", "one.txt"],
      ["documents", "two.txt"],
      ["avatar", "replacement.png"],
    ]);
    expect(avatar.proposals).toContain(null);
    expect(avatar.proposals[avatar.proposals.length - 1]).toMatch(/^018f47c1-/u);
    expect(documents.proposals[documents.proposals.length - 1]).toEqual([
      expect.stringMatching(/^018f47c1-/u),
      expect.stringMatching(/^018f47c1-/u),
    ]);
    fixture.manager.dispose();
  });

  it("accepts zero-byte files and never turns directory or path claims into transport paths", async () => {
    const fixture = manager();
    const owner = island("oddities");
    await fixture.manager.select({ field: "attachment", input: input(), island: owner.port }, [
      file("..\\private/zero.txt", 0),
    ]);

    expect(
      fixture.transport.requests.filter(({ operation }) => operation === "put_chunk"),
    ).toHaveLength(0);
    const create = fixture.transport.requests.find(({ operation }) => operation === "create");
    expect(create).toMatchObject({
      file: { name: "zero.txt", size: 0 },
      operation: "create",
    });
    expect(JSON.stringify(create)).not.toContain("private");
    expect(fixture.manager.snapshot().uploads[0]).toMatchObject({
      name: "zero.txt",
      state: "ready",
    });
    fixture.manager.dispose();
  });

  it("bounds active files, chunk size, retained chunks, and all current-document secrets", async () => {
    const transport = new MemoryTransport();
    const fixture = manager(transport, 2);
    const owner = island("bounded");
    await fixture.manager.select(
      { field: "documents", input: input(true), island: owner.port },
      Array.from({ length: 6 }, (_, index) => file(`${String(index)}.bin`, 700 * KIB)),
    );

    const chunks = transport.requests.filter(
      (request): request is Extract<UploadTransportRequest, { operation: "put_chunk" }> =>
        request.operation === "put_chunk",
    );
    expect(transport.maximumActive).toBeLessThanOrEqual(2);
    expect(chunks.every(({ bytes }) => bytes.byteLength <= 256 * KIB)).toBe(true);
    expect(
      fixture.manager.snapshot().uploads.every(({ retainedChunks }) => retainedChunks <= 2),
    ).toBe(true);
    expect(fixture.manager.inspectSecrets()).toEqual({ chunks: 0, files: 6, grants: 6 });

    fixture.manager.dispose();
    expect(fixture.manager.inspectSecrets()).toEqual({ chunks: 0, files: 0, grants: 0 });
  });

  it("clears the typed proposal when canceling, removing, or rejecting a replacement", async () => {
    const fixture = manager();
    const owner = island("controls");
    const control = input();
    await fixture.manager.select({ field: "avatar", input: control, island: owner.port }, [
      file("avatar.png", 1),
    ]);
    await fixture.manager.cancel(owner.port, "avatar");
    expect(owner.proposals[owner.proposals.length - 1]).toBeNull();

    await fixture.manager.select({ field: "avatar", input: control, island: owner.port }, [
      file("avatar.png", 1),
    ]);
    await fixture.manager.remove(owner.port, "avatar");
    expect(owner.proposals[owner.proposals.length - 1]).toBeNull();
    expect(control.value).toBe("");

    const rejected = file("too-many.bin", 1);
    await fixture.manager.select(
      { field: "avatar", input: control, island: owner.port },
      Array.from({ length: 65 }, () => rejected),
    );
    expect(owner.proposals[owner.proposals.length - 1]).toBeNull();
    fixture.manager.dispose();
  });

  it("contains proposal callback failures and retires deterministically", async () => {
    const fixture = manager();
    const owner = island("hostile");
    owner.port.proposeUploadHandle = vi.fn(() => {
      throw new Error("secret-proposal-error");
    });
    await expect(
      fixture.manager.select({ field: "avatar", input: input(), island: owner.port }, [
        file("avatar.png", 1),
      ]),
    ).resolves.toBeUndefined();
    expect(fixture.manager.snapshot().uploads[0]?.state).toBe("failed");
    fixture.manager.dispose();
  });
});
