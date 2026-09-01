#!/usr/bin/env node

// Rule contract for the macro expansion and compile budget. The rules must
// hold on any machine, under any CARGO_BUILD_JOBS setting, and alongside
// unrelated load: expansion size is deterministic and is compared against the
// checked baseline, while isolated check time is compared only within one run.

import assert from "node:assert/strict";

import {
  assertBaseline,
  assertCheckTimeBounded,
  assertLinearExpansion,
  cargoBuildJobs,
  maxCheckTimeRatio,
  maxExpansionGrowth,
} from "../scripts/expansion-budget-rules.mjs";

function fixture(componentCount, overrides = {}) {
  return {
    component_count: componentCount,
    expanded_tokens: 1_700 * componentCount + 62,
    expanded_bytes: 9_100 * componentCount + 1_074,
    cargo_check_milliseconds: 5_400,
    fixture_sha256: `digest-${componentCount}`,
    ...overrides,
  };
}

function corpus(overridesByCount = {}) {
  return [1, 10, 100].map((count) => fixture(count, overridesByCount[count]));
}

const baseline = {
  schema_version: 1,
  workload: "component-expansion",
  fixtures: corpus(),
};

// Expansion size is deterministic, so the checked baseline bounds it.
assert.doesNotThrow(() => assertBaseline({ fixtures: corpus() }, baseline));
assert.throws(
  () =>
    assertBaseline(
      { fixtures: corpus({ 100: { fixture_sha256: "edited" } }) },
      baseline,
    ),
  /100-component fixture drifted/,
);
assert.throws(
  () =>
    assertBaseline(
      { fixtures: corpus({ 10: { expanded_tokens: 17_062 * 1.2 } }) },
      baseline,
    ),
  /10-component expanded_tokens regressed by more than 10%/,
);
assert.throws(
  () =>
    assertBaseline(
      { fixtures: corpus({ 1: { expanded_bytes: 10_174 * 1.2 } }) },
      baseline,
    ),
  /1-component expanded_bytes regressed by more than 10%/,
);

// Isolated check time on the developer machine is exploratory evidence. A
// slow machine, a capped CARGO_BUILD_JOBS, or an unrelated build next door
// must never fail the gate against the checked baseline's milliseconds.
assert.doesNotThrow(() =>
  assertBaseline(
    {
      fixtures: corpus({
        1: { cargo_check_milliseconds: 54_000 },
        10: { cargo_check_milliseconds: 54_000 },
        100: { cargo_check_milliseconds: 54_000 },
      }),
    },
    baseline,
  ),
);

// Dependency compilation dominates the isolated check, so a per-component
// compile regression shows up as the larger fixtures taking a multiple of the
// 1-component fixture in the same run. The same-run ratio cancels machine
// speed, job count, and concurrent load.
assert.equal(maxCheckTimeRatio, 2);
assert.doesNotThrow(() =>
  assertCheckTimeBounded(
    corpus({
      1: { cargo_check_milliseconds: 11_650 },
      10: { cargo_check_milliseconds: 11_897 },
      100: { cargo_check_milliseconds: 12_465 },
    }),
  ),
);
assert.doesNotThrow(() =>
  assertCheckTimeBounded(
    corpus({
      1: { cargo_check_milliseconds: 5_000 },
      100: { cargo_check_milliseconds: 10_000 },
    }),
  ),
);
assert.throws(
  () =>
    assertCheckTimeBounded(
      corpus({
        1: { cargo_check_milliseconds: 5_000 },
        100: { cargo_check_milliseconds: 12_500 },
      }),
    ),
  /100-component isolated cargo check took 2\.50x the 1-component check in the same run/,
);
assert.throws(
  () =>
    assertCheckTimeBounded(
      corpus({
        1: { cargo_check_milliseconds: 5_000 },
        10: { cargo_check_milliseconds: 10_001 },
      }),
    ),
  /10-component isolated cargo check took 2\.00x the 1-component check in the same run/,
);

// Expansion growth across 1, 10, and 100 components stays linear: ten times
// the components may cost at most twelve times the tokens or bytes.
assert.equal(maxExpansionGrowth, 12);
assert.doesNotThrow(() => assertLinearExpansion(corpus()));
assert.throws(
  () =>
    assertLinearExpansion(
      corpus({ 100: { expanded_tokens: fixture(10).expanded_tokens * 13 } }),
    ),
  /expanded_tokens grew 13\.00x from 10 to 100 components/,
);
assert.throws(
  () =>
    assertLinearExpansion(
      corpus({ 10: { expanded_bytes: fixture(1).expanded_bytes * 12.5 } }),
    ),
  /expanded_bytes grew 12\.50x from 1 to 10 components/,
);
// Check time is owned by the same-run bound, not by the growth rule.
assert.doesNotThrow(() =>
  assertLinearExpansion(
    corpus({
      1: { cargo_check_milliseconds: 1_000 },
      10: { cargo_check_milliseconds: 13_000 },
    }),
  ),
);

// The recorded environment names the job setting the measurement ran under.
assert.equal(cargoBuildJobs({ CARGO_BUILD_JOBS: "2" }), "2");
assert.equal(cargoBuildJobs({}), "default");

process.stdout.write("expansion budget rules ok\n");
