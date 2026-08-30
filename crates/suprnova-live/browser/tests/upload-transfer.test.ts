import { describe, expect, it } from "vitest";

import { FetchUploadTransport } from "../src/uploads/feature.js";
import { UploadTransfer, uploadSha256HexForTest } from "../src/uploads/transfer.js";
import type {
  UploadConnectivity,
  UploadRandomness,
  UploadTransport,
  UploadTransportRequest,
  UploadTransportResponse,
} from "../src/uploads/types.js";

const HANDLE = "018f47c1-2af0-7cc4-a001-000000000001";

class Connectivity implements UploadConnectivity {
  connected = true;

  online(): boolean {
    return this.connected;
  }
}

class Randomness implements UploadRandomness {
  #next = 0;

  idempotencyKey(): string {
    this.#next += 1;
    return `retry-${String(this.#next)}`;
  }
}

class InterruptedTransport implements UploadTransport {
  readonly requests: UploadTransportRequest[] = [];
  failChunk = true;
  expireComplete = false;
  hangCancel = false;
  revision = 1n;

  send(request: UploadTransportRequest): Promise<UploadTransportResponse> {
    this.requests.push(request);
    if (request.operation === "create") {
      return Promise.resolve({ grant: "secret", handle: HANDLE, revision: "1", state: "queued" });
    }
    if (request.operation === "put_chunk" && this.failChunk) {
      this.failChunk = false;
      return Promise.reject(new TypeError("network unavailable"));
    }
    if (request.operation === "complete" && this.expireComplete) {
      return Promise.reject(Object.assign(new Error("expired"), { code: "upload_expired" }));
    }
    if (request.operation === "cancel" && this.hangCancel) return new Promise(() => undefined);
    this.revision += 1n;
    return Promise.resolve({
      revision: this.revision.toString(),
      state:
        request.operation === "complete"
          ? "ready"
          : request.operation === "cancel"
            ? "canceled"
            : "transferring",
    });
  }
}

describe("bounded upload transfer", () => {
  it("computes standard incremental SHA-256 without retaining the whole file", () => {
    expect(uploadSha256HexForTest(new ArrayBuffer(0))).toBe(
      "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    );
    expect(uploadSha256HexForTest(new TextEncoder().encode("abc").buffer)).toBe(
      "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    );
  });

  it("keeps transfer grants in authorization headers and out of URLs", async () => {
    const calls: { input: RequestInfo | URL; init: RequestInit | undefined }[] = [];
    const fetchPort: typeof globalThis.fetch = (input, init) => {
      calls.push({ input, init });
      return Promise.resolve(
        new Response(JSON.stringify({ revision: "2", state: "transferring" }), {
          headers: { "Content-Type": "application/json" },
          status: 200,
        }),
      );
    };
    const transport = new FetchUploadTransport(fetchPort);
    await transport.send({
      bytes: new Uint8Array([1]).buffer,
      checksum: "a".repeat(64),
      chunkIndex: 0,
      expectedRevision: "1",
      grant: "secret-transfer-grant",
      handle: HANDLE,
      idempotencyKey: "chunk-1",
      operation: "put_chunk",
      signal: new AbortController().signal,
    });

    expect(calls[0]?.input).toBe("/__live/upload");
    const requestInput = calls[0]?.input;
    if (typeof requestInput !== "string") throw new Error("expected string upload endpoint");
    expect(requestInput).not.toContain("secret-transfer-grant");
    const headers = new Headers(calls[0]?.init?.headers);
    expect(headers.get("Authorization")).toBe("SuprnovaUpload secret-transfer-grant");
    expect(calls[0]?.init?.body).toBeInstanceOf(ArrayBuffer);
  });

  it("rejects an oversized response body before JSON allocation", async () => {
    const transport = new FetchUploadTransport(() =>
      Promise.resolve(
        new Response("x".repeat(16 * 1024 + 1), {
          headers: { "Content-Type": "application/json" },
          status: 200,
        }),
      ),
    );
    await expect(
      transport.send({
        field: "avatar",
        file: { lastModified: 0, name: "avatar.png", size: 0, type: "image/png" },
        idempotencyKey: "create-1",
        island: { component: "fixture.upload", documentKey: "doc", slot: "slot" },
        operation: "create",
        signal: new AbortController().signal,
      }),
    ).rejects.toThrow("upload_transport_failed");
  });

  it("retries an uncertain chunk with identical bytes and idempotency identity", async () => {
    const transport = new InterruptedTransport();
    const connectivity = new Connectivity();
    const proposals: unknown[] = [];
    const transfer = new UploadTransfer({
      chunkBytes: 4,
      connectivity,
      field: "avatar",
      file: new File([new Uint8Array([1, 2, 3, 4, 5])], "avatar.bin"),
      island: {
        element: { nodeType: 1 } as Element,
        identity: Object.freeze({ component: "fixture.upload", documentKey: "doc", slot: "slot" }),
        proposeUploadHandle(_field, proposal) {
          proposals.push(proposal);
          return "accepted";
        },
      },
      randomness: new Randomness(),
      transport,
    });

    await transfer.run();
    expect(transfer.snapshot().state).toBe("interrupted");
    const first = transport.requests.find(
      (request): request is Extract<UploadTransportRequest, { operation: "put_chunk" }> =>
        request.operation === "put_chunk",
    );
    expect(first).toBeDefined();
    expect(transfer.snapshot().retainedChunks).toBe(1);

    await transfer.retry();
    const chunks = transport.requests.filter(
      (request): request is Extract<UploadTransportRequest, { operation: "put_chunk" }> =>
        request.operation === "put_chunk",
    );
    expect(chunks[1]?.idempotencyKey).toBe(first?.idempotencyKey);
    expect([...new Uint8Array(chunks[1]?.bytes ?? new ArrayBuffer(0))]).toEqual([
      ...new Uint8Array(first?.bytes ?? new ArrayBuffer(0)),
    ]);
    expect(transfer.snapshot()).toMatchObject({ retainedChunks: 0, state: "ready" });
    expect(proposals).toEqual([HANDLE]);
    transfer.dispose();
  });

  it("does not begin while offline and allows explicit retry once connectivity returns", async () => {
    const transport = new InterruptedTransport();
    transport.failChunk = false;
    const connectivity = new Connectivity();
    connectivity.connected = false;
    const transfer = new UploadTransfer({
      chunkBytes: 4,
      connectivity,
      field: "avatar",
      file: new File([new Uint8Array([1])], "avatar.bin"),
      island: {
        element: { nodeType: 1 } as Element,
        identity: Object.freeze({ component: "fixture.upload", documentKey: "doc", slot: "slot" }),
        proposeUploadHandle: () => "accepted",
      },
      randomness: new Randomness(),
      transport,
    });

    await transfer.run();
    expect(transfer.snapshot().state).toBe("interrupted");
    expect(transport.requests).toHaveLength(0);
    connectivity.connected = true;
    await transfer.retry();
    expect(transfer.snapshot().state).toBe("ready");
    transfer.dispose();
  });

  it("clears the typed proposal and every secret when server authority expires", async () => {
    const transport = new InterruptedTransport();
    transport.failChunk = false;
    transport.expireComplete = true;
    const proposals: unknown[] = [];
    const transfer = new UploadTransfer({
      chunkBytes: 4,
      connectivity: new Connectivity(),
      field: "avatar",
      file: new File([], "avatar.bin"),
      island: {
        element: { nodeType: 1 } as Element,
        identity: Object.freeze({ component: "fixture.upload", documentKey: "doc", slot: "slot" }),
        proposeUploadHandle(_field, proposal) {
          proposals.push(proposal);
          return "accepted";
        },
      },
      randomness: new Randomness(),
      transport,
    });

    await transfer.run();
    expect(transfer.snapshot().state).toBe("expired");
    expect(proposals).toEqual([HANDLE, null]);
    expect(transfer.inspectSecrets()).toEqual({ chunks: 0, files: 0, grants: 0 });
  });

  it("treats a typed terminal chunk response as authoritative and stops immediately", async () => {
    const proposals: unknown[] = [];
    const requests: UploadTransportRequest[] = [];
    const transport: UploadTransport = {
      send(request) {
        requests.push(request);
        return Promise.resolve(
          request.operation === "create"
            ? { grant: "secret", handle: HANDLE, revision: "1", state: "queued" }
            : { revision: "2", state: "expired" },
        );
      },
    };
    const transfer = new UploadTransfer({
      chunkBytes: 1,
      connectivity: new Connectivity(),
      field: "avatar",
      file: new File([new Uint8Array([1, 2, 3])], "avatar.bin"),
      island: {
        element: { nodeType: 1 } as Element,
        identity: Object.freeze({ component: "fixture.upload", documentKey: "doc", slot: "slot" }),
        proposeUploadHandle(_field, proposal) {
          proposals.push(proposal);
          return "accepted";
        },
      },
      randomness: new Randomness(),
      transport,
    });

    await transfer.run();

    expect(requests.map(({ operation }) => operation)).toEqual(["create", "put_chunk"]);
    expect(transfer.snapshot()).toMatchObject({ handle: null, state: "expired" });
    expect(transfer.inspectSecrets()).toEqual({ chunks: 0, files: 0, grants: 0 });
    expect(proposals).toEqual([HANDLE, null]);
  });

  it("reconciles an uncertain completion through status and reuses its idempotency key", async () => {
    const requests: UploadTransportRequest[] = [];
    let completionAttempts = 0;
    const transport: UploadTransport = {
      send(request) {
        requests.push(request);
        if (request.operation === "create") {
          return Promise.resolve({
            grant: "secret",
            handle: HANDLE,
            revision: "1",
            state: "queued",
          });
        }
        if (request.operation === "complete") {
          completionAttempts += 1;
          if (completionAttempts === 1) return Promise.reject(new TypeError("response lost"));
          return Promise.resolve({ revision: "2", state: "verifying" });
        }
        if (request.operation === "status") {
          return Promise.resolve({ nextChunkIndex: 0, revision: "1", state: "transferring" });
        }
        return Promise.resolve({ revision: "2", state: "transferring" });
      },
    };
    const transfer = new UploadTransfer({
      chunkBytes: 4,
      connectivity: new Connectivity(),
      field: "avatar",
      file: new File([], "avatar.bin"),
      island: {
        element: { nodeType: 1 } as Element,
        identity: Object.freeze({ component: "fixture.upload", documentKey: "doc", slot: "slot" }),
        proposeUploadHandle: () => "accepted",
      },
      randomness: new Randomness(),
      transport,
    });

    await transfer.run();
    expect(transfer.snapshot().state).toBe("interrupted");
    await transfer.retry();

    const completions = requests.filter(
      (request): request is Extract<UploadTransportRequest, { operation: "complete" }> =>
        request.operation === "complete",
    );
    expect(requests.map(({ operation }) => operation)).toEqual([
      "create",
      "complete",
      "status",
      "complete",
    ]);
    expect(completions[1]?.idempotencyKey).toBe(completions[0]?.idempotencyKey);
    expect(transfer.snapshot().state).toBe("verifying");
    transfer.dispose();
  });

  it("rejects operation-state mismatches and regressing revisions", async () => {
    const transfer = new UploadTransfer({
      chunkBytes: 4,
      connectivity: new Connectivity(),
      field: "avatar",
      file: new File([new Uint8Array([1])], "avatar.bin"),
      island: {
        element: { nodeType: 1 } as Element,
        identity: Object.freeze({ component: "fixture.upload", documentKey: "doc", slot: "slot" }),
        proposeUploadHandle: () => "accepted",
      },
      randomness: new Randomness(),
      transport: {
        send(request) {
          return Promise.resolve(
            request.operation === "create"
              ? { grant: "secret", handle: HANDLE, revision: "1", state: "queued" }
              : { revision: "1", state: "queued" },
          );
        },
      },
    });

    await transfer.run();
    expect(transfer.snapshot().state).toBe("failed");
    transfer.dispose();
  });

  it("never exposes grants through snapshots or errors and clears them on cancel", async () => {
    const transport = new InterruptedTransport();
    transport.failChunk = false;
    const proposals: unknown[] = [];
    const transfer = new UploadTransfer({
      chunkBytes: 4,
      connectivity: new Connectivity(),
      field: "avatar",
      file: new File([new Uint8Array([1])], "avatar.bin"),
      island: {
        element: { nodeType: 1 } as Element,
        identity: Object.freeze({ component: "fixture.upload", documentKey: "doc", slot: "slot" }),
        proposeUploadHandle(_field, proposal) {
          proposals.push(proposal);
          return "accepted";
        },
      },
      randomness: new Randomness(),
      transport,
    });
    await transfer.run();
    expect(JSON.stringify(transfer.snapshot())).not.toContain("secret");
    expect(transfer.inspectSecrets()).toEqual({ chunks: 0, files: 1, grants: 1 });
    transport.hangCancel = true;
    await expect(transfer.cancel()).resolves.toBeUndefined();
    expect(proposals[proposals.length - 1]).toBeNull();
    expect(transfer.inspectSecrets()).toEqual({ chunks: 0, files: 0, grants: 0 });
  });
});
