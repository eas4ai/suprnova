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
