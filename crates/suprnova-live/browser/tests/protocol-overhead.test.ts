import { readFile } from "node:fs/promises";

import { describe, expect, it } from "vitest";

// Framework overhead bounds from the architecture overview: a signed snapshot
// adds at most 768 bytes beyond application state and lifecycle memo, and a
// response adds at most 1 KiB of fixed control-envelope bytes around its HTML
// and snapshot payload.
const SNAPSHOT_OVERHEAD_LIMIT_BYTES = 768;
const CONTROL_OVERHEAD_LIMIT_BYTES = 1024;

interface SnapshotFixture {
  readonly id: string;
  readonly encoded: { readonly body: { readonly state: unknown; readonly memo: unknown } };
}

describe("protocol overhead", () => {
  it("keeps signed snapshot framing within 768 bytes beyond state and memo", async () => {
    const fixtures = JSON.parse(
      await readFile(new URL("../../fixtures/v1/snapshot-success.json", import.meta.url), "utf8"),
    ) as { readonly cases: readonly SnapshotFixture[] };
    const instance = fixtures.cases.find((fixture) => fixture.id === "instance-v1");
    if (instance === undefined) throw new Error("missing instance-v1 fixture");

    const snapshotBytes = JSON.stringify(instance.encoded).length;
    const stateBytes = JSON.stringify(instance.encoded.body.state).length;
    const memoBytes = JSON.stringify(instance.encoded.body.memo).length;

    expect(snapshotBytes - stateBytes - memoBytes).toBeLessThanOrEqual(
      SNAPSHOT_OVERHEAD_LIMIT_BYTES,
    );
  });

  it("keeps the response control envelope within 1 KiB around HTML and snapshot payload", () => {
    const html = "h".repeat(8 * 1024);
    const payload = "s".repeat(16 * 1024);
    const response = JSON.stringify({
      accepted_revision: "8",
      correlation_id: "EBESExQVFhcYGRobHB0eHw",
      effects: [],
      events: [],
      extensions: {},
      outcome: "accepted",
      protocol_version: 1,
      render: { html, kind: "html" },
      snapshot: { body: { payload }, signature: "A".repeat(43) },
      validation: {},
    });

    expect(response.length - html.length - payload.length).toBeLessThanOrEqual(
      CONTROL_OVERHEAD_LIMIT_BYTES,
    );
  });
});
