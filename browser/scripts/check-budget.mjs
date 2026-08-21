import { readFile } from "node:fs/promises";

const fixtureUrl = new URL("../../fixtures/v1/snapshot-success.json", import.meta.url);
const fixtures = JSON.parse(await readFile(fixtureUrl, "utf8"));
const instance = fixtures.cases.find((fixture) => fixture.id === "instance-v1");
if (!instance) throw new Error("missing instance-v1 budget fixture");

const snapshot = JSON.stringify(instance.encoded);
const stateBytes = JSON.stringify(instance.encoded.body.state).length;
const memoBytes = JSON.stringify(instance.encoded.body.memo).length;
const snapshotOverhead = snapshot.length - stateBytes - memoBytes;
if (snapshotOverhead > 768) {
  throw new Error(`snapshot overhead ${snapshotOverhead} exceeds 768 bytes`);
}

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
const controlOverhead = response.length - html.length - payload.length;
if (controlOverhead > 1024) {
  throw new Error(`control overhead ${controlOverhead} exceeds 1024 bytes`);
}

console.log(`budget ok control_overhead=${controlOverhead} snapshot_overhead=${snapshotOverhead}`);
