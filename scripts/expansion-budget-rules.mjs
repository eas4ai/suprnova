// Rules for the macro expansion and isolated compile budget.
//
// Expansion size is deterministic for a pinned toolchain, so tokens and bytes
// are compared against the checked baseline and must grow linearly across the
// 1-, 10-, and 100-component fixtures. Isolated `cargo check` time on a
// developer machine is exploratory evidence: it depends on machine speed,
// CARGO_BUILD_JOBS, and whatever else is running, so it is never compared
// against the checked baseline's milliseconds. Dependency compilation
// dominates that check, so a per-component compile regression large enough to
// matter shows up as the larger fixtures taking a multiple of the 1-component
// fixture in the same run, and the same-run ratio cancels every one of those
// environmental factors.

// Ten times the components may cost at most twelve times the tokens or bytes.
export const maxExpansionGrowth = 12;

// Each larger fixture's isolated check must finish within this multiple of
// the 1-component fixture's check in the same run.
export const maxCheckTimeRatio = 2;

export function assertLinearExpansion(fixtures) {
  for (const metric of ["expanded_tokens", "expanded_bytes"]) {
    for (let index = 1; index < fixtures.length; index += 1) {
      const ratio = fixtures[index][metric] / fixtures[index - 1][metric];
      if (ratio > maxExpansionGrowth) {
        throw new Error(
          `${metric} grew ${ratio.toFixed(2)}x from ${fixtures[index - 1].component_count} to ${fixtures[index].component_count} components`,
        );
      }
    }
  }
}

export function assertCheckTimeBounded(fixtures) {
  const reference = fixtures[0];
  for (const fixture of fixtures.slice(1)) {
    const ratio =
      fixture.cargo_check_milliseconds / reference.cargo_check_milliseconds;
    if (ratio > maxCheckTimeRatio) {
      throw new Error(
        `${fixture.component_count}-component isolated cargo check took ${ratio.toFixed(2)}x the ${reference.component_count}-component check in the same run`,
      );
    }
  }
}

export function assertBaseline(observed, baseline) {
  if (
    baseline.schema_version !== 1 ||
    baseline.workload !== "component-expansion"
  ) {
    throw new Error("checked expansion baseline has an unsupported schema");
  }
  for (const fixture of observed.fixtures) {
    const expected = baseline.fixtures.find(
      (candidate) => candidate.component_count === fixture.component_count,
    );
    if (!expected || expected.fixture_sha256 !== fixture.fixture_sha256) {
      throw new Error(
        `${fixture.component_count}-component fixture drifted; regenerate and review the baseline`,
      );
    }
    for (const metric of ["expanded_tokens", "expanded_bytes"]) {
      if (fixture[metric] > expected[metric] * 1.1) {
        throw new Error(
          `${fixture.component_count}-component ${metric} regressed by more than 10%`,
        );
      }
    }
  }
}

export function cargoBuildJobs(environment) {
  return environment.CARGO_BUILD_JOBS ?? "default";
}
