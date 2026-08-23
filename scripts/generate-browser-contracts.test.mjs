import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import * as generator from "./generate-browser-contracts.mjs";

const loadContracts =
  generator.loadContracts ??
  (() => assert.fail("missing schema 2 contract loader"));
const validateV4Evolution =
  generator.validateV4Evolution ??
  (() => assert.fail("missing v4 evolution validator"));
const partitionRuntimeContracts =
  generator.partitionRuntimeContracts ??
  (() => assert.fail("missing runtime contract partitioner"));

const fixture = JSON.parse(
  await readFile(
    new URL("../fixtures/v4/directive-grammar.json", import.meta.url),
    "utf8",
  ),
);
const previousFixture = JSON.parse(
  await readFile(
    new URL("../fixtures/v3/directive-grammar.json", import.meta.url),
    "utf8",
  ),
);

function changed(mutator) {
  const value = structuredClone(fixture);
  mutator(value);
  return value;
}

test("schema 2 rejects unknown fields and capabilities", () => {
  assert.throws(
    () =>
      loadContracts(
        changed((grammar) => (grammar.directives[0].endpoint = "/chosen")),
      ),
    /unknown_directive_0_field_endpoint/,
  );
  assert.throws(
    () =>
      loadContracts(
        changed((grammar) => (grammar.directives[41].capability = "custom@1")),
      ),
    /invalid_capability_upload/,
  );
});

test("schema 2 rejects duplicate or ambiguous roles and unknown conflicts", () => {
  assert.throws(
    () =>
      loadContracts(
        changed(
          (grammar) => (grammar.directives[41].roles = ["cancel", "cancel"]),
        ),
      ),
    /duplicate_upload_roles/,
  );
  assert.throws(
    () =>
      loadContracts(
        changed((grammar) => {
          grammar.directives[41].modifiers = ["cancel"];
        }),
      ),
    /ambiguous_upload_suffix_cancel/,
  );
  assert.throws(
    () =>
      loadContracts(
        changed(
          (grammar) => (grammar.directives[41].conflicts = ["not-registered"]),
        ),
      ),
    /unknown_upload_conflict_not-registered/,
  );
});

test("v4 evolution preserves every v3 contract and promotes only four reviewed names", () => {
  const contracts = loadContracts(fixture);
  assert.doesNotThrow(() =>
    validateV4Evolution(previousFixture, fixture, contracts),
  );
  const changedFixture = changed((grammar) => {
    grammar.directives[0].fallback = "inert";
  });
  assert.throws(
    () =>
      validateV4Evolution(
        previousFixture,
        changedFixture,
        loadContracts(changedFixture),
      ),
    /changed_v3_directive_click/,
  );
});

test("runtime contracts partition completely from descriptor capabilities", () => {
  const contracts = loadContracts(fixture);
  const partition = partitionRuntimeContracts(contracts);
  assert.deepEqual(
    partition.core.map(({ name }) => name),
    contracts
      .filter(({ capability }) => capability === null)
      .map(({ name }) => name),
  );
  assert.deepEqual(
    partition.features.map(({ name }) => name),
    contracts
      .filter(({ capability }) => capability !== null)
      .map(({ name }) => name),
  );
  assert.deepEqual(
    partition.coreReservedNames,
    [...partition.features]
      .sort((left, right) => {
        if (left.capability < right.capability) return -1;
        if (left.capability > right.capability) return 1;
        return 0;
      })
      .map(({ name }) => name),
  );
  assert.equal(
    partition.core.length + partition.features.length,
    contracts.length,
  );

  const capabilityDriven = loadContracts(
    changed((grammar) => {
      grammar.directives[0].capability = "async@1";
    }),
  );
  const changedPartition = partitionRuntimeContracts(capabilityDriven);
  assert.equal(
    changedPartition.core.some(({ name }) => name === "click"),
    false,
  );
  assert.equal(
    changedPartition.features.some(({ name }) => name === "click"),
    true,
  );
  assert.equal(changedPartition.coreReservedNames.includes("click"), true);
});

test("generated TypeScript exposes separate core and feature runtime lookups", async () => {
  const source = await readFile(
    new URL("../browser/src/generated/directive-contract.ts", import.meta.url),
    "utf8",
  );
  assert.match(source, /export type RuntimeDirectiveContract = readonly \[/u);
  assert.match(source, /export type FeatureDirectiveContract = readonly \[/u);
  assert.match(source, /export const CORE_RESERVED_DIRECTIVES =/u);
  assert.match(source, /export function featureDirectiveContract\(/u);
});
