import { describe, expect, it } from "vitest";

import { decodeUploadProtocolOperation, UploadProtocolError } from "../src/uploads/protocol.js";
import { UploadProtocolStateError, UploadProtocolStateMachine } from "../src/uploads/state.js";

const HANDLE = "018f47c1-2af0-7cc4-a001-000000000001";

describe("production upload protocol authority", () => {
  it("decodes the canonical wire through the same validator used by the runtime", () => {
    expect(
      decodeUploadProtocolOperation(
        `{"handle":"${HANDLE}","operation":"status","protocol_version":1}`,
      ),
    ).toMatchObject({ operation: "status" });

    for (const [encoded, code] of [
      [`{"handle":"${HANDLE}","operation":"status","protocol_version":2}`, "unsupported_protocol"],
      [`{"operation":"status","protocol_version":1,"protocol_version":1}`, "duplicate_field"],
      [`{"operation":"status","protocol_version":1,"snapshot":{}}`, "unknown_field"],
    ] as const) {
      try {
        decodeUploadProtocolOperation(encoded);
        throw new Error("upload_protocol_case_accepted");
      } catch (error: unknown) {
        expect(error).toBeInstanceOf(UploadProtocolError);
        expect((error as UploadProtocolError).code).toBe(code);
      }
    }
  });

  it("derives state and retry outcomes through the production transition authority", () => {
    const machine = new UploadProtocolStateMachine("created", 1n);
    const request = Object.freeze({
      expectedRevision: 1n,
      idempotencyKey: "queue-created",
      transition: "queue" as const,
    });
    expect(machine.apply(request)).toEqual({
      disposition: "applied",
      revision: 2n,
      state: "queued",
    });
    expect(machine.apply(request)).toEqual({
      disposition: "existing_outcome",
      revision: 2n,
      state: "queued",
    });
  });

  it("retains only the production-bounded idempotency outcome history", () => {
    const machine = new UploadProtocolStateMachine("transferring", 0n);
    for (let revision = 0n; revision < 64n; revision += 1n) {
      machine.apply({
        expectedRevision: revision,
        idempotencyKey: `chunk-${String(revision)}`,
        transition: "put_chunk",
      });
    }

    expect(() =>
      machine.apply({
        expectedRevision: 64n,
        idempotencyKey: "chunk-over-bound",
        transition: "put_chunk",
      }),
    ).toThrow(
      expect.objectContaining<Partial<UploadProtocolStateError>>({
        code: "upload_idempotency_history_full",
      }),
    );
  });
});
