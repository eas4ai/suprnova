import { describe, expect, it, vi } from "vitest";

import { reacquireUpload } from "../src/uploads/resume.js";
import { uploadFileIdentity } from "../src/uploads/types.js";

const HANDLE = "018f47c1-2af0-7cc4-a001-000000000001";

describe("application-owned upload reacquisition", () => {
  it("accepts only an application-issued grant for the exact user-held File identity", async () => {
    const selected = new File([new Uint8Array([1, 2, 3])], "avatar.png", {
      lastModified: 1_700_000_000_000,
      type: "image/png",
    });
    const reacquire = vi.fn(() =>
      Promise.resolve({
        fileIdentity: uploadFileIdentity(selected),
        grant: "new-secret-grant",
        nextChunkIndex: 1,
        revision: "4",
        state: "transferring" as const,
        uploadedBytes: 1,
      }),
    );

    const result = await reacquireUpload(
      { reacquire },
      { field: "avatar", file: selected, handle: HANDLE },
    );

    expect(reacquire).toHaveBeenCalledWith({
      field: "avatar",
      fileIdentity: uploadFileIdentity(selected),
      handle: HANDLE,
    });
    expect(result.handle).toBe(HANDLE);
    expect(result.file).toBe(selected);
    expect(result.nextChunkIndex).toBe(1);
    expect(result.uploadedBytes).toBe(1);
  });

  it("rejects identity drift and has no built-in endpoint or ambient persistence", async () => {
    const selected = new File([new Uint8Array([1])], "avatar.png", { lastModified: 7 });
    const stores = ["localStorage", "sessionStorage", "indexedDB"] as const;
    const reads = vi.fn();
    const descriptors = stores.map(
      (name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)] as const,
    );
    for (const name of stores) {
      Object.defineProperty(globalThis, name, { configurable: true, get: reads });
    }
    try {
      await expect(
        reacquireUpload(
          {
            reacquire() {
              return Promise.resolve({
                fileIdentity: { ...uploadFileIdentity(selected), size: 2 },
                grant: "new-secret-grant",
                nextChunkIndex: 1,
                revision: "4",
                state: "transferring",
                uploadedBytes: 1,
              });
            },
          },
          { field: "avatar", file: selected, handle: HANDLE },
        ),
      ).rejects.toThrow("upload_reacquire_identity_mismatch");
      expect(reads).not.toHaveBeenCalled();
    } finally {
      for (const [name, descriptor] of descriptors) {
        if (descriptor === undefined) Reflect.deleteProperty(globalThis, name);
        else Object.defineProperty(globalThis, name, descriptor);
      }
    }
  });

  it("fails closed when no application port is supplied", async () => {
    await expect(
      reacquireUpload(undefined, {
        field: "avatar",
        file: new File([], "avatar.png"),
        handle: HANDLE,
      }),
    ).rejects.toThrow("upload_reacquire_unavailable");
  });

  it("rejects a malformed authoritative chunk cursor", async () => {
    const selected = new File([new Uint8Array([1])], "avatar.png");
    await expect(
      reacquireUpload(
        {
          reacquire: () =>
            Promise.resolve({
              fileIdentity: uploadFileIdentity(selected),
              grant: "new-secret-grant",
              nextChunkIndex: -1,
              revision: "4",
              state: "transferring",
              uploadedBytes: 1,
            }),
        },
        { field: "avatar", file: selected, handle: HANDLE },
      ),
    ).rejects.toThrow("upload_next_chunk_index_invalid");
  });
});
