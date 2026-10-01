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
const renderTypeScript =
  generator.renderTypeScript ??
  (() => assert.fail("missing TypeScript contract renderer"));

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

const valueGrammar = {
  token: {
    maximum_bytes: 64,
    initial: "ascii_lowercase",
    continuation: ["ascii_lowercase", "ascii_digit", "_", ".", ":", "-"],
  },
  integer: {
    canonical: true,
    maximum_absolute: "9007199254740991",
  },
};

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

test("schema 2 requires exact ordered syntax vocabularies", () => {
  for (const field of [
    "target_kinds",
    "literal_kinds",
    "argument_forms",
    "value_kinds",
    "fallbacks",
  ]) {
    assert.throws(
      () =>
        loadContracts(
          changed((grammar) => {
            grammar.syntax[field] = [...grammar.syntax[field]].reverse();
          }),
        ),
      new RegExp(`invalid_${field}`),
    );
  }
});

test("schema 2 carries one closed bounded scalar-value grammar", () => {
  assert.deepEqual(fixture.syntax.value_grammar, valueGrammar);
  assert.throws(
    () =>
      loadContracts(
        changed((grammar) => {
          grammar.syntax.value_grammar = structuredClone(valueGrammar);
          grammar.syntax.value_grammar.token.maximum_bytes = 65;
        }),
      ),
    /invalid_value_grammar/,
  );
  assert.throws(
    () =>
      loadContracts(
        changed((grammar) => {
          grammar.syntax.value_grammar = structuredClone(valueGrammar);
          grammar.syntax.value_grammar.token.continuation.reverse();
        }),
      ),
    /invalid_value_grammar/,
  );
});

test("schema 2 rejects modifiers deeper than the runtime parser", () => {
  assert.throws(
    () =>
      loadContracts(
        changed((grammar) => {
          grammar.directives[0].modifiers = ["one.two.three.four"];
        }),
      ),
    /too_deep_click_modifiers/,
  );
  assert.throws(
    () =>
      loadContracts(
        changed((grammar) => {
          grammar.model_modifiers.push("one.two.three.four");
        }),
      ),
    /too_deep_model_modifiers/,
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

test("schema 2 carries closed modifier-conflict groups", () => {
  const withModifierConflicts = changed((grammar) => {
    for (const directive of grammar.directives) {
      directive.modifier_conflicts = [];
    }
    grammar.directives[43].modifier_conflicts = [
      ["visible", "always"],
      ["5s", "15s", "30s", "60s"],
    ];
    grammar.directives[44].modifier_conflicts = [["push-only", "hybrid"]];
  });
  const contracts = loadContracts(withModifierConflicts);
  assert.deepEqual(contracts[43].modifierConflicts, [
    ["visible", "always"],
    ["5s", "15s", "30s", "60s"],
  ]);
  assert.deepEqual(contracts[44].modifierConflicts, [["push-only", "hybrid"]]);

  assert.throws(
    () =>
      loadContracts(
        changed((grammar) => {
          for (const directive of grammar.directives) {
            directive.modifier_conflicts = [];
          }
          grammar.directives[43].modifier_conflicts = [["visible", "missing"]];
        }),
      ),
    /unknown_poll_modifier_conflict_missing/,
  );
  assert.throws(
    () =>
      loadContracts(
        changed((grammar) => {
          for (const directive of grammar.directives) {
            directive.modifier_conflicts = [];
          }
          grammar.directives[43].modifier_conflicts = [
            ["visible", "always"],
            ["always", "5s"],
          ];
        }),
      ),
    /duplicate_poll_modifier_conflict_always/,
  );
});

test("schema 2 emits fixture-owned freshness policy without a handwritten mapping", () => {
  const policyChanged = changed((grammar) => {
    const combination = grammar.freshness_combinations.find(
      ({ poll, stream }) => poll === true && stream === "default",
    );
    combination.result = "directive_conflict";
    grammar.freshness_combinations.reverse();
  });

  const rendered = renderTypeScript(
    policyChanged,
    "a".repeat(64),
    loadContracts(policyChanged),
  );

  assert.match(rendered, /\[true, "default", "directive_conflict"\]/);
  assert.ok(
    rendered.indexOf('[true, "push-only", "directive_conflict"]') <
      rendered.indexOf('[false, "absent", "none"]'),
  );
});

test("schema 2 structurally closes freshness coverage and legal result shapes", () => {
  for (const mutate of [
    (grammar) => grammar.freshness_combinations.pop(),
    (grammar) => {
      grammar.freshness_combinations[0] = structuredClone(
        grammar.freshness_combinations[1],
      );
    },
    (grammar) => {
      grammar.freshness_combinations[0].poll = "false";
    },
    (grammar) => {
      grammar.freshness_combinations[0].stream = "websocket";
    },
    (grammar) => {
      grammar.freshness_combinations[0].result = "stale";
    },
    (grammar) => {
      grammar.freshness_combinations[0].result = "poll_only";
    },
    (grammar) => {
      const combination = grammar.freshness_combinations.find(
        ({ poll, stream }) => poll === true && stream === "push-only",
      );
      combination.result = "hybrid_poll_override";
    },
  ]) {
    assert.throws(
      () => loadContracts(changed(mutate)),
      /invalid_freshness_combinations/,
    );
  }
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

  const reorderedPrevious = structuredClone(previousFixture);
  reorderedPrevious.syntax.fallbacks.reverse();
  assert.throws(
    () => validateV4Evolution(reorderedPrevious, fixture, contracts),
    /invalid_fallbacks/,
  );

  const reorderedCurrent = changed((grammar) =>
    grammar.syntax.fallbacks.reverse(),
  );
  assert.throws(
    () => validateV4Evolution(previousFixture, reorderedCurrent, contracts),
    /invalid_fallbacks/,
  );

  const divergentFixture = changed((grammar) => {
    grammar.directives[42].fallback = "native";
  });
  assert.throws(
    () =>
      validateV4Evolution(
        previousFixture,
        fixture,
        loadContracts(divergentFixture),
      ),
    /inconsistent_v4_contracts/,
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
  assert.match(source, /export function freshnessCombination\(/u);
});
