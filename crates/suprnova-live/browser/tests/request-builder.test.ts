import { describe, expect, it } from "vitest";

import type { JsonValue } from "../src/canonical.js";
import type { IslandMetadata } from "../src/islands/metadata.js";
import type { IslandRecord } from "../src/islands/record.js";
import type { RuntimeRandomness } from "../src/runtime/ports.js";
import type { IntentSource, ServerOperation } from "../src/scheduler/intent.js";
import { createParamsChangedIntent, ServerIntent } from "../src/scheduler/intent.js";
import {
  DOCUMENT_KEY_EXTENSION,
  LiveRequestBuilder,
  type LiveRequestBuildInput,
} from "../src/transport/request.js";

const ENVELOPE = Object.freeze({
  body: Object.freeze({}),
  signature: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
});

function randomness(seed = 0): RuntimeRandomness {
  let next = seed;
  return {
    randomBytes(length) {
      const bytes = new Uint8Array(length);
      for (let index = 0; index < length; index += 1) bytes[index] = (next + index) & 0xff;
      next += length;
      return bytes;
    },
  };
}

function metadata(form: "seed" | "instance"): IslandMetadata {
  return Object.freeze({
    component: "catalog.search",
    documentKey: "primary",
    instanceId: form === "instance" ? "sLGys7S1tre4ubq7vL2-vw" : null,
    lazyComplete: false,
    protocolMinimum: 1,
    revision: form === "instance" ? 7n : 0n,
    runtimeContract: 1,
    slot: "search-results",
    snapshot: ENVELOPE,
    snapshotForm: form,
  });
}

function intent(
  form: "seed" | "instance",
  operations: readonly ServerOperation[],
  proposals: Readonly<Record<string, JsonValue>> = Object.freeze({}),
): ServerIntent {
  const source = Object.freeze({
    directive: Object.freeze({ name: "click", value: "search" }),
    element: Object.freeze({}),
    eventType: "click",
    island: Object.freeze({ metadata: metadata(form) }) as unknown as IslandRecord,
    trusted: true,
  }) as unknown as IntentSource;
  return new ServerIntent(
    source,
    operations,
    form === "seed" ? "UFFSU1RVVldYWVpbXF1eXw" : null,
    proposals,
    Object.freeze(
      Object.fromEntries(Object.keys(proposals).map((field, index) => [field, BigInt(index + 1)])),
    ),
  );
}

function input(
  version: 1 | 2,
  value: ServerIntent,
  overrides: Partial<LiveRequestBuildInput> = {},
): LiveRequestBuildInput {
  return {
    intent: value,
    protocolVersion: version,
    randomness: randomness(),
    ...overrides,
  };
}

describe("Live request builder", () => {
  it("carries a queued signed child capability into one params-changed request", async () => {
    const envelope = Object.freeze({
      body: Object.freeze({ parameters: Object.freeze({ query: "rust" }) }),
      signature: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    });
    const record = Object.freeze({
      element: Object.freeze({}),
      metadata: metadata("instance"),
    }) as unknown as IslandRecord;
    const parentSnapshot = Object.freeze({
      body: Object.freeze({ revision: "7" }),
      signature: "PPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPP",
    });
    const request = await new LiveRequestBuilder().build(
      input(2, createParamsChangedIntent(record, envelope, parentSnapshot)),
    );
    const parsed = JSON.parse(request.text) as Record<string, unknown>;
    expect(parsed["child_parameters"]).toEqual({ envelope, parent_snapshot: parentSnapshot });
    expect(parsed["snapshot"]).toEqual({ envelope: record.metadata.snapshot, kind: "instance" });
    expect(parsed["operations"]).toEqual([{ kind: "params_changed" }]);
  });

  it("constructs every v1 and v2 snapshot/action/lifecycle form through validation", async () => {
    const action = Object.freeze({
      arguments: Object.freeze({}),
      kind: "invoke_action" as const,
      name: "search",
    });
    const cases: LiveRequestBuildInput[] = [
      input(1, intent("seed", [action])),
      input(1, intent("instance", [action])),
      input(2, intent("seed", [action])),
      input(2, intent("instance", [action])),
      input(2, intent("instance", [Object.freeze({ kind: "params_changed" })]), {
        childParameters: Object.freeze({
          envelope: Object.freeze({
            body: Object.freeze({ parameters: Object.freeze({ query: "rust" }) }),
            signature: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
          }),
          parent_snapshot: Object.freeze({
            body: Object.freeze({ revision: "7" }),
            signature: "PPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPP",
          }),
        }),
      }),
      input(2, intent("instance", [Object.freeze({ kind: "lazy_complete" })])),
      input(2, intent("instance", [Object.freeze({ kind: "fresh_render" })])),
    ];

    for (const fixture of cases) {
      const request = await new LiveRequestBuilder().build(fixture);
      const parsed = JSON.parse(request.text) as Record<string, unknown>;
      expect(parsed["protocol_version"]).toBe(fixture.protocolVersion);
      expect((parsed["extensions"] as Record<string, unknown>)[DOCUMENT_KEY_EXTENSION]).toBe(
        "primary",
      );
      expect(request.identity.baseRevision).toBe(fixture.intent.source.island.metadata.revision);
      expect(request.identity.correlationId).toMatch(/^[A-Za-z0-9_-]{22}$/u);
      expect(request.identity.idempotencyKey).toMatch(/^[A-Za-z0-9_-]{22}$/u);
      expect(request.identity.semanticDigest).toMatch(/^[A-Za-z0-9_-]{43}$/u);
    }
  });

  it("keeps immutable identity for compatible rebuilds and rotates it for semantic changes", async () => {
    const value = intent("instance", [
      Object.freeze({ arguments: Object.freeze({}), kind: "invoke_action", name: "search" }),
    ]);
    const builder = new LiveRequestBuilder();
    const identitySource = randomness();
    const first = await builder.build(input(2, value, { randomness: identitySource }));
    const compatible = await builder.build(input(2, value, { randomness: identitySource }));
    const changed = await builder.build(
      input(2, value, {
        extensions: Object.freeze({ x_example_v1: "changed" }),
        randomness: identitySource,
      }),
    );

    expect(compatible.identity).toEqual(first.identity);
    expect(compatible.text).toBe(first.text);
    expect(changed.identity.idempotencyKey).not.toBe(first.identity.idempotencyKey);
    expect(changed.identity.semanticDigest).not.toBe(first.identity.semanticDigest);

    const lifecycle = intent("instance", [Object.freeze({ kind: "params_changed" })]);
    const firstChild = await builder.build(
      input(2, lifecycle, {
        childParameters: Object.freeze({
          envelope: Object.freeze({ body: Object.freeze({ query: "rust" }) }),
          parent_snapshot: ENVELOPE,
        }),
        randomness: identitySource,
      }),
    );
    const changedChild = await builder.build(
      input(2, lifecycle, {
        childParameters: Object.freeze({
          envelope: Object.freeze({ body: Object.freeze({ query: "zig" }) }),
          parent_snapshot: ENVELOPE,
        }),
        randomness: identitySource,
      }),
    );
    expect(changedChild.identity.idempotencyKey).not.toBe(firstChild.identity.idempotencyKey);
    expect(changedChild.identity.semanticDigest).not.toBe(firstChild.identity.semanticDigest);
  });

  it("rejects wrong protocol forms, reserved extension forgery, and protocol bounds", async () => {
    const action = Object.freeze({
      arguments: Object.freeze({}),
      kind: "invoke_action" as const,
      name: "search",
    });
    await expect(
      new LiveRequestBuilder().build(input(1, intent("instance", [{ kind: "fresh_render" }]))),
    ).rejects.toThrow("request_protocol_form");
    await expect(
      new LiveRequestBuilder().build(
        input(2, intent("instance", [action]), {
          extensions: Object.freeze({ [DOCUMENT_KEY_EXTENSION]: "other" }),
        }),
      ),
    ).rejects.toThrow("request_document_key_conflict");

    const operations = Array.from({ length: 9 }, () =>
      Object.freeze({
        arguments: Object.freeze({}),
        kind: "invoke_action" as const,
        name: "search",
      }),
    );
    await expect(
      new LiveRequestBuilder().build(input(2, intent("instance", operations))),
    ).rejects.toThrow("too_many_operations");

    const proposalOperations = Array.from({ length: 9 }, (_, index) =>
      Object.freeze({ field: `field_${String(index)}`, kind: "sync_model" as const }),
    );
    const excessiveProposals = Object.freeze(
      Object.fromEntries(proposalOperations.map((operation) => [operation.field, operation.field])),
    );
    await expect(
      new LiveRequestBuilder().build(
        input(2, intent("instance", proposalOperations, excessiveProposals)),
      ),
    ).rejects.toThrow("too_many_model_proposals");

    await expect(
      new LiveRequestBuilder().build(
        input(2, intent("instance", [action]), {
          extensions: Object.freeze(
            Object.fromEntries(
              Array.from({ length: 8 }, (_, index) => [`x_test_${String(index)}`, index]),
            ),
          ),
        }),
      ),
    ).rejects.toThrow("request_extension_limit");

    await expect(
      new LiveRequestBuilder().build({
        ...input(2, intent("instance", [action])),
        protocolVersion: 3,
      } as unknown as LiveRequestBuildInput),
    ).rejects.toThrow("unsupported_protocol_version");
  });

  it("rejects a historical raw child-parameter envelope from the v2 admission path", async () => {
    await expect(
      new LiveRequestBuilder().build(
        input(2, intent("instance", [Object.freeze({ kind: "params_changed" })]), {
          childParameters: Object.freeze({
            body: Object.freeze({ parameters: Object.freeze({ query: "rust" }) }),
            signature: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
          }),
        }),
      ),
    ).rejects.toThrow("invalid_protocol_envelope");
  });

  it("closes a seed intent when identity randomness fails", async () => {
    const value = intent("seed", [
      Object.freeze({ arguments: Object.freeze({}), kind: "invoke_action", name: "search" }),
    ]);
    let finishes = 0;
    value.onFinish(() => {
      finishes += 1;
    });

    await expect(
      new LiveRequestBuilder().build({
        intent: value,
        protocolVersion: 1,
        randomness: {
          randomBytes() {
            throw new Error("hostile_randomness");
          },
        },
      }),
    ).rejects.toThrow("request_identity_unavailable");
    expect(finishes).toBe(1);
    expect(value.promotionNonce()).toBeNull();
  });
});
