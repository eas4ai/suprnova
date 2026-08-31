# Suprnova Live Cutover Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move the committed Suprnova Live product and specification history into the Suprnova workspace as one buildable internal crate tree with one authoritative checker and gate path.

**Architecture:** Import the standalone branch as a history-preserving subtree under `crates/suprnova-live/`, then make that tree a normal member of the Suprnova workspace without introducing an engine-to-framework dependency. Keep the nested Live scripts rooted at their own crate directory, expose them to Suprnova through one wrapper in the existing ignored local `scripts/` tooling repository, and leave the standalone checkout as immutable historical evidence after the cutover. The public Suprnova branch does not publish that private release tooling.

**Tech Stack:** Git subtree, Cargo workspace resolver 3, Rust 2024, Node.js ESM, strict TypeScript, npm lockfile, existing Suprnova and Live shell gates.

---

## File structure

- `Cargo.toml` - owns Suprnova workspace membership, shared release version, edition, MSRV, and license.
- `Cargo.lock` - one integrated dependency resolution for Suprnova and Live.
- `crates/suprnova-live/` - imported engine, browser runtime, tests, fixtures, benchmarks, specifications, checker, and implementation documentation.
- `crates/suprnova-live/Cargo.toml` - internal engine package manifest after removal of the nested workspace declaration.
- `crates/suprnova-live/crates/*/Cargo.toml` - retained macro-development, facade-fixture, and test-support packages aligned to workspace package policy.
- `crates/suprnova-live/scripts/gate.sh` - Live-owned gate that remains runnable from any current directory and targets only integrated Live packages unless a test deliberately invokes the parent framework.
- `/home/shawn/workspace2/suprnova/scripts/check-suprnova-live.sh` - local Suprnova-owned unattended adapter that invokes the integrated Live gate with the correct profile and no duplicate implementation.
- `/home/shawn/workspace2/suprnova/scripts/gate-steps.json` - existing ignored local gate registry with one ordinary Live step using the adapter.
- `/home/shawn/workspace2/suprnova/scripts/gate-assets.json` and local smoke contracts - integrity and installation ownership for the private gate addition.
- `crates/suprnova-live/AGENTS.md` - integrated local rules and active Iteration 005 path.
- `crates/suprnova-live/docs/specs/suprnova-live/conventions.md` - authoritative integrated paths and verification commands.

## Task 1: Import the committed standalone history

**Files:**

- Create: `crates/suprnova-live/**` through `git subtree add`
- Verify: `crates/suprnova-live/docs/specs/suprnova-live/iterations/005.md`
- Verify: `crates/suprnova-live/src/lib.rs`

- [ ] **Step 1: Verify the exact source branch and cleanliness boundary**

Run:

```bash
rtk git -C /home/shawn/workspace2/suprnova-live rev-parse iteration-004-uploads-async
rtk git -C /home/shawn/workspace2/suprnova-live show --stat --oneline 6d19d02
rtk git status --short --branch
```

Expected: the branch resolves to `6d19d02`; the Suprnova integration worktree is clean; only committed files can enter the subtree command.

- [ ] **Step 2: Import without squashing history**

Run:

```bash
rtk git subtree add --prefix=crates/suprnova-live /home/shawn/workspace2/suprnova-live iteration-004-uploads-async
```

Expected: Git creates one subtree merge commit, `crates/suprnova-live/` exists, and the imported ancestry includes `6d19d02` and `3293ed5`.

- [ ] **Step 3: Prove protected local files did not cross the boundary**

Run:

```bash
rtk git status --short
rtk git ls-files crates/suprnova-live/node_modules crates/suprnova-live/docs/specs/suprnova-live/iterations/next crates/suprnova-live/docs/implementation/local-reactivity.md crates/suprnova-live/browser/e2e/bootstrap.spec.ts
rtk git show HEAD:crates/suprnova-live/docs/specs/suprnova-live/iterations/005.md
```

Expected: the worktree is clean after the subtree commit; `node_modules` and untracked `iterations/next/` are absent; the two tracked files equal their committed source forms rather than standalone uncommitted edits; Iteration 005 is present.

## Task 2: Reconcile the Cargo workspace

**Files:**

- Modify: `Cargo.toml`
- Modify: `crates/suprnova-live/Cargo.toml`
- Modify: `crates/suprnova-live/crates/suprnova-live-macros/Cargo.toml`
- Modify: `crates/suprnova-live/crates/suprnova-live-macro-fixture/Cargo.toml`
- Modify: `crates/suprnova-live/crates/suprnova-live-test-support/Cargo.toml`
- Modify: `Cargo.lock`

- [ ] **Step 1: Add a metadata test that describes the required package topology**

Create `crates/suprnova-live/tests/integrated_workspace.rs`:

```rust
use std::{env, path::PathBuf, process::Command};

use serde_json::Value;

#[test]
fn live_packages_share_the_suprnova_workspace_without_a_framework_cycle() {
    let workspace_root = PathBuf::from(
        env::var("SUPRNOVA_WORKSPACE_ROOT")
            .expect("SUPRNOVA_WORKSPACE_ROOT must identify the integration worktree"),
    );
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(&workspace_root)
        .output()
        .expect("cargo metadata must start");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let metadata: Value =
        serde_json::from_slice(&output.stdout).expect("cargo metadata must be valid JSON");
    let packages = metadata["packages"]
        .as_array()
        .expect("metadata packages must be an array");
    let framework_version = packages
        .iter()
        .find(|package| package["name"] == "suprnova")
        .and_then(|package| package["version"].as_str())
        .expect("the public suprnova package must be present");
    let live_root = workspace_root.join("crates/suprnova-live");

    for name in [
        "suprnova-live",
        "suprnova-live-macros",
        "suprnova-live-macro-fixture",
        "suprnova-live-test-support",
    ] {
        let package = packages
            .iter()
            .find(|package| package["name"] == name)
            .unwrap_or_else(|| panic!("missing integrated package {name}"));
        assert_eq!(package["version"], framework_version);
        let manifest = PathBuf::from(
            package["manifest_path"]
                .as_str()
                .expect("manifest_path must be a string"),
        );
        assert!(manifest.starts_with(&live_root));
    }

    let engine = packages
        .iter()
        .find(|package| package["name"] == "suprnova-live")
        .expect("the integrated engine must be present");
    let has_framework_dependency = engine["dependencies"]
        .as_array()
        .expect("dependencies must be an array")
        .iter()
        .any(|dependency| dependency["name"] == "suprnova");
    assert!(!has_framework_dependency);
}
```

The dynamic comparison with the public framework package version keeps the test
valid across later Suprnova releases while proving all internal packages share
one release identity.

- [ ] **Step 2: Run the topology test and verify the precondition fails**

Run:

```bash
rtk env SUPRNOVA_WORKSPACE_ROOT=/home/shawn/workspace2/suprnova-live-integration CARGO_INCREMENTAL=0 cargo test --manifest-path crates/suprnova-live/Cargo.toml --test integrated_workspace
```

Expected: FAIL because the imported manifest still declares a nested workspace and uses standalone package metadata.

- [ ] **Step 3: Add the Live packages to the root workspace**

Extend the root `members` list with:

```toml
"crates/suprnova-live",
"crates/suprnova-live/crates/suprnova-live-macro-fixture",
"crates/suprnova-live/crates/suprnova-live-macros",
"crates/suprnova-live/crates/suprnova-live-test-support",
```

Extend the root workspace with:

```toml
exclude = ["crates/suprnova-live/fuzz"]
```

- [ ] **Step 4: Remove the nested workspace and inherit package policy**

Replace each imported package's literal `version`, `edition`, `rust-version`, and `license` with:

```toml
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
```

Delete the `[workspace]` table from `crates/suprnova-live/Cargo.toml`. Keep `publish = false`, descriptions, exact behavioral dependency pins, feature declarations, lint tables, and benchmark declarations unchanged.

- [ ] **Step 5: Resolve the unified lockfile**

Run:

```bash
rtk cargo metadata --format-version 1 --no-deps
rtk env CARGO_INCREMENTAL=0 cargo check -p suprnova-live --all-targets --all-features
```

Expected: metadata succeeds with one workspace root; the Live engine checks under Rust 1.94.0 and the shared lockfile.

- [ ] **Step 6: Run the topology test and focused package tests**

Run:

```bash
rtk env SUPRNOVA_WORKSPACE_ROOT=/home/shawn/workspace2/suprnova-live-integration CARGO_INCREMENTAL=0 cargo test -p suprnova-live --test integrated_workspace
rtk env CARGO_INCREMENTAL=0 cargo test -p suprnova-live-macros --test ui
rtk env CARGO_INCREMENTAL=0 cargo test -p suprnova-live-test-support --test reference_host -- --test-threads=1
```

Expected: PASS.

- [ ] **Step 7: Commit the workspace reconciliation**

Run GitNexus change detection, review its affected files, then commit only the manifests, lockfile, and topology test:

```bash
rtk git add Cargo.toml Cargo.lock crates/suprnova-live/Cargo.toml crates/suprnova-live/crates/suprnova-live-macros/Cargo.toml crates/suprnova-live/crates/suprnova-live-macro-fixture/Cargo.toml crates/suprnova-live/crates/suprnova-live-test-support/Cargo.toml crates/suprnova-live/tests/integrated_workspace.rs
rtk git commit -m "build: integrate suprnova live workspace"
```

## Task 3: Make Live verification relocation-safe

**Files:**

- Modify: `crates/suprnova-live/scripts/gate.sh`
- Modify: `crates/suprnova-live/tests/gate_contract.sh`
- Modify: `crates/suprnova-live/tests/documentation_contract.sh`
- Modify: `crates/suprnova-live/scripts/check-msrv.sh` or the exact MSRV owner found during impact analysis
- Modify: any imported script whose contract assumes the standalone directory is a Cargo workspace root
- Test: imported script contract tests

- [ ] **Step 1: Run the imported gate contract from the integrated path**

Run:

```bash
rtk proxy crates/suprnova-live/tests/gate_contract.sh
```

Expected: FAIL at each stale standalone-root or Rust 1.91.1 assumption, providing the regression list to fix.

- [ ] **Step 2: Add one explicit crate-root and workspace-root split**

At the top of Live-owned shell scripts, resolve the crate root from the script path and the Suprnova workspace root from `git rev-parse --show-toplevel`. Use the crate root for specs, browser files, fixtures, benchmarks, and local documentation; use the workspace root only for Cargo workspace commands and the shared lockfile.

The scripts must not depend on the caller's current directory and must reject a crate root outside the resolved workspace.

- [ ] **Step 3: Replace workspace-wide Cargo invocations with explicit Live package scope**

The relocated ordinary Live gate must target the four Live packages and the excluded fuzz manifest explicitly. It must not run all Suprnova packages as a side effect of `--workspace`. Retain all existing Live tests, doctests, Clippy review, fuzz build, browser gates, budgets, and qualification behavior.

- [ ] **Step 4: Bind MSRV to the Suprnova workspace**

Replace the literal `+1.91.1` integrated checks with the workspace MSRV `1.94.0`, and add a contract assertion that the Live package manifests use `rust-version.workspace = true`. Do not introduce blanket `-D warnings`; retain the narrow `clippy::disallowed_methods` denial already justified by the Live correctness contract.

- [ ] **Step 5: Verify relocation from both directories**

Run:

```bash
rtk proxy crates/suprnova-live/tests/gate_contract.sh
rtk node crates/suprnova-live/scripts/check-specs.mjs
rtk proxy crates/suprnova-live/tests/documentation_contract.sh
rtk env CARGO_INCREMENTAL=0 cargo test -p suprnova-live --test golden_fixtures --test browser_contract_properties
```

Then run the same spec and documentation commands with the current directory set to `crates/suprnova-live/`.

Expected: PASS from both locations with identical authoritative files.

- [ ] **Step 6: Commit the relocation-safe Live gate**

Run GitNexus change detection, then commit the affected imported scripts and contract tests:

```bash
rtk git add crates/suprnova-live/scripts crates/suprnova-live/tests/gate_contract.sh crates/suprnova-live/tests/documentation_contract.sh
rtk git commit -m "build: relocate suprnova live verification"
```

## Task 4: Integrate the Live gate into Suprnova

**Files:**

- Create locally: `/home/shawn/workspace2/suprnova/scripts/check-suprnova-live.sh`
- Modify locally: `/home/shawn/workspace2/suprnova/scripts/gate-steps.json`
- Modify locally as required by the existing tooling contract: `/home/shawn/workspace2/suprnova/scripts/gate-assets.json`
- Test locally: `/home/shawn/workspace2/suprnova/scripts/tests/release-normal-smoke.sh`

The entire `/home/shawn/workspace2/suprnova/scripts/` directory is an existing
gitignored nested Git repository. Task 4 commits there locally and never pushes
or adds its files to the public Suprnova worktree.

- [ ] **Step 1: Preserve the pre-existing local-tooling boundary**

Verify the nested tooling repository status separately from the public
Suprnova worktree. Checkpoint the developer-approved completed release-tooling
changes in their own local commit before adding Live, excluding caches and
other generated workstation residue. Run GitNexus impact where the local index
can resolve a symbol; shell registry entries with no indexed symbol require
literal contract review rather than a false low-risk claim.

- [ ] **Step 2: Write the failing wrapper contract**

Add a failing local smoke assertion and run it before creating the adapter.
The assertion must require exactly one registered ordinary Live step, the
adapter asset in the local integrity registry, and release-install preservation.

```bash
rtk proxy scripts/tests/release-normal-smoke.sh
```

Expected: FAIL because the wrapper and registered step do not yet exist.

Create the local `scripts/check-suprnova-live.sh` adapter:

```bash
#!/usr/bin/env bash
set -euo pipefail

repository_root=$(git rev-parse --show-toplevel)
live_gate=${repository_root}/crates/suprnova-live/scripts/gate.sh

if [[ ! -x "${live_gate}" ]]; then
    printf 'integrated Suprnova Live gate is missing or not executable: %s\n' "${live_gate}" >&2
    exit 1
fi

export SUPRNOVA_LIVE_RELEASE=${SUPRNOVA_LIVE_RELEASE:-0}
exec "${live_gate}"
```

- [ ] **Step 3: Register the ordinary local gate step**

Add one `suprnova-live` step to the local `gate-steps.json` registry for the
`default` and `full` tiers. Invoke only `scripts/check-suprnova-live.sh`; do not
duplicate Live commands in the registry or runner. Register the adapter in the
existing gate asset/integrity mechanism and update the local installation and
smoke contracts so a release-tool reinstall cannot silently remove it.

The registry command is exactly:

```json
{"argv": ["scripts/check-suprnova-live.sh"]}
```

- [ ] **Step 4: Verify the adapter against the integration worktree**

Expose the local tooling repository to the integration worktree only through
an ignored temporary link or equivalent reversible local setup, then run the
wrapper with the integration worktree as the current Git root. Do not copy the
private tooling into public version control.

Run:

```bash
rtk proxy scripts/check-suprnova-live.sh
rtk proxy scripts/tests/release-normal-smoke.sh
rtk proxy bash -n scripts/check-suprnova-live.sh
rtk git diff --check
```

Expected: the ordinary integrated Live gate passes from the integration
worktree, local gate installation/smoke contracts pass, and qualification-only
failures remain confined to explicit release mode.

- [ ] **Step 5: Commit the local gate adapter without pushing**

Run change detection and review within the nested local tooling repository,
then create a local-only commit. Do not push and do not stage the ignored
`scripts/` path in the public Suprnova worktree.

```bash
rtk git -C /home/shawn/workspace2/suprnova/scripts add check-suprnova-live.sh gate-steps.json gate-assets.json tests
rtk git -C /home/shawn/workspace2/suprnova/scripts commit -m "build: gate integrated suprnova live"
```

## Task 5: Update authority and path contracts

**Files:**

- Modify: `crates/suprnova-live/AGENTS.md`
- Modify: `crates/suprnova-live/docs/specs/suprnova-live/00-overview.md`
- Modify: `crates/suprnova-live/docs/specs/suprnova-live/conventions.md`
- Modify: imported implementation documents that name `/home/shawn/workspace2/suprnova-live` as the active authority
- Modify: imported package metadata or README files that claim standalone operation as the product location

- [ ] **Step 1: Locate stale authority claims**

Use `tilth` over `crates/suprnova-live/docs`, `crates/suprnova-live/AGENTS.md`, package manifests, and scripts for literal standalone paths and phrases such as `dedicated workspace`, `active Suprnova checkout`, and `eventual destination`.

Classify every match as historical evidence, a command that must be relocated, or stale authority prose. Historical Iteration 001 through 004 statements remain unchanged unless they falsely describe current authority outside their historical context.

- [ ] **Step 2: Update current authority without rewriting history**

Set the integrated path as the sole maintained product/spec/checker authority. Preserve the standalone repository and its commits as historical provenance. Update current verification commands to run from the Suprnova root or integrated crate root and retain the explicit Iteration 004 qualification blockers.

- [ ] **Step 3: Run specification and documentation gates**

Run:

```bash
rtk node crates/suprnova-live/scripts/check-specs.mjs
rtk node crates/suprnova-live/scripts/check-implementation-docs.mjs
rtk git diff --check
```

Expected: PASS with no stale current-authority claim and no changed historical claim unless required for truth.

- [ ] **Step 4: Commit the authority cutover documentation**

```bash
rtk git add crates/suprnova-live/AGENTS.md crates/suprnova-live/docs crates/suprnova-live/Cargo.toml crates/suprnova-live/browser/package.json
rtk git commit -m "docs: establish integrated live authority"
```

## Task 6: Verify the cutover as a coherent build

**Files:**

- Verify only unless a discovered defect requires a focused regression fix

- [ ] **Step 1: Run focused Rust checks**

Run sequentially:

```bash
rtk env CARGO_INCREMENTAL=0 cargo fmt --all -- --check
rtk env CARGO_INCREMENTAL=0 cargo clippy -p suprnova-live -p suprnova-live-macros -p suprnova-live-macro-fixture -p suprnova-live-test-support --all-targets --all-features
rtk env CARGO_INCREMENTAL=0 cargo test -p suprnova-live --all-targets --all-features --no-fail-fast
rtk env CARGO_INCREMENTAL=0 cargo test -p suprnova-live-macros --all-targets --all-features --no-fail-fast
rtk env CARGO_INCREMENTAL=0 cargo test -p suprnova-live-test-support --all-targets --all-features --no-fail-fast
rtk env CARGO_INCREMENTAL=0 cargo test -p suprnova-live --doc --all-features
```

Expected: PASS with reviewed warnings and no blanket warning denial.

- [ ] **Step 2: Run browser checks from the integrated package**

Run:

```bash
rtk npm --prefix crates/suprnova-live/browser ci
rtk npm --prefix crates/suprnova-live/browser run generate:check
rtk npm --prefix crates/suprnova-live/browser run format:check
rtk npm --prefix crates/suprnova-live/browser run lint
rtk npm --prefix crates/suprnova-live/browser run typecheck
rtk npm --prefix crates/suprnova-live/browser test
rtk npm --prefix crates/suprnova-live/browser run build
rtk npm --prefix crates/suprnova-live/browser run build:check
rtk npm --prefix crates/suprnova-live/browser run budget
```

Expected: PASS; deterministic artifact hashes and reviewed baseline history remain unchanged unless a reviewed source change required a new candidate.

- [ ] **Step 3: Run the integrated ordinary gate**

Run:

```bash
rtk env SUPRNOVA_LIVE_RELEASE=0 CARGO_INCREMENTAL=0 scripts/check-suprnova-live.sh
```

Expected: PASS. Do not use `SUPRNOVA_LIVE_RELEASE=1` to claim qualification until S1/B1 evidence and the Iteration 004 historical-baseline decision are resolved.

- [ ] **Step 4: Run change-impact and drift review**

Run GitNexus `detect_changes` against `main`, review every affected symbol and process, and run:

```bash
rtk tilth diff main..HEAD --blast --budget 12000
rtk git diff --check main..HEAD
rtk git status --short --branch
```

Expected: only the subtree import, workspace integration, gate adapter, and authority/path corrections are present. No Magnetar file changes and no unrelated Suprnova refactor appear.

- [ ] **Step 5: Record the cutover checkpoint**

If verification required no further edits, no final empty commit is needed. Record the exact passing commands and remaining Iteration 004 qualification blockers in the Iteration 005 implementation ledger before starting the separate framework-facade and RenderCache implementation plans inside the same iteration.
