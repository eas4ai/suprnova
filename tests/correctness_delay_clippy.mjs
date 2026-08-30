#!/usr/bin/env node

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repositoryRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);
const sourceConfig = path.join(repositoryRoot, "clippy.toml");
assert.equal(
  fs.existsSync(sourceConfig),
  true,
  "the compiler-resolved correctness-delay policy must live in clippy.toml",
);

const fixtureRoot = fs.mkdtempSync(
  path.join(os.tmpdir(), "suprnova-live-correctness-delay-clippy-"),
);
try {
  fs.mkdirSync(path.join(fixtureRoot, "src"), { recursive: true });
  fs.copyFileSync(sourceConfig, path.join(fixtureRoot, "clippy.toml"));
  fs.writeFileSync(
    path.join(fixtureRoot, "Cargo.toml"),
    `[package]
name = "correctness-delay-clippy-fixture"
version = "0.0.0"
edition = "2024"
publish = false

[features]
absolute = []
extern_alias = []
glob = []
std_alias = []
tokio_alias = []

[dependencies]
tokio = { version = "=1.53.1", features = ["rt", "time"] }

[workspace]
`,
    "utf8",
  );
  fs.writeFileSync(
    path.join(fixtureRoot, "src/lib.rs"),
    `#![allow(dead_code, missing_docs)]

#[cfg(feature = "absolute")]
pub fn absolute_path() {
    ::std::hint::spin_loop();
}

#[cfg(feature = "extern_alias")]
extern crate std as standard;

#[cfg(feature = "extern_alias")]
pub fn extern_alias() {
    standard::thread::yield_now();
}

#[cfg(feature = "glob")]
pub fn glob_import() {
    use std::thread::*;
    yield_now();
}

#[cfg(feature = "std_alias")]
pub fn standard_alias() {
    use std::thread::sleep as pause;
    pause(std::time::Duration::ZERO);
}

#[cfg(feature = "tokio_alias")]
pub async fn tokio_alias() {
    use tokio::time::sleep as pause;
    pause(std::time::Duration::ZERO).await;
}

mod local {
    pub fn sleep() {}
}

pub fn local_shadowing_is_not_std_sleep() {
    use local::sleep;
    sleep();
}

#[allow(
    clippy::disallowed_methods,
    reason = "failure-only watchdog bounds a deliberately noncooperative fixture"
)]
pub fn reasoned_watchdog_allow_is_narrow() {
    std::thread::sleep(std::time::Duration::ZERO);
}
`,
    "utf8",
  );

  const runClippy = (features) =>
    spawnSync(
      "cargo",
      [
        "clippy",
        "--manifest-path",
        path.join(fixtureRoot, "Cargo.toml"),
        "--no-default-features",
        ...(features.length > 0 ? ["--features", features.join(",")] : []),
        "--",
        "-D",
        "clippy::disallowed_methods",
      ],
      {
        encoding: "utf8",
        env: {
          ...process.env,
          CARGO_INCREMENTAL: "0",
          CARGO_NET_OFFLINE: "true",
          CARGO_TARGET_DIR: path.join(fixtureRoot, "target"),
          CARGO_TERM_COLOR: "never",
        },
      },
    );

  for (const feature of [
    "absolute",
    "extern_alias",
    "glob",
    "std_alias",
    "tokio_alias",
  ]) {
    const rejected = runClippy([feature]);
    const diagnostic = `${rejected.stdout}${rejected.stderr}`;
    assert.notEqual(rejected.status, 0, `${feature} must fail Clippy`);
    assert.match(
      diagnostic,
      /disallowed method/u,
      `${feature} must fail through compiler-resolved disallowed_methods`,
    );
  }

  const accepted = runClippy([]);
  assert.equal(
    accepted.status,
    0,
    `local shadowing and a narrow reasoned allow must pass:\n${accepted.stdout}${accepted.stderr}`,
  );
} finally {
  fs.rmSync(fixtureRoot, { force: true, recursive: true });
}

process.stdout.write("correctness-delay Clippy self-test ok\n");
