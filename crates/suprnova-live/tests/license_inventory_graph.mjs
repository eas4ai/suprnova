#!/usr/bin/env node

import assert from "node:assert/strict";

import {
  cargoDependencyClosure,
  loadLockedCargoMetadata,
  resolveGitWorkspaceRoot,
} from "../scripts/license-inventory-cargo.mjs";

const liveRoots = [
  "suprnova-live",
  "suprnova-live-macros",
  "suprnova-live-macro-fixture",
  "suprnova-live-test-support",
];

function cargoPackage(id, name) {
  return {
    id,
    name,
    version: "1.0.0",
    license: "MIT",
    source: "registry+test",
  };
}

function resolveNode(id, dependencies = []) {
  return { id, dependencies };
}

const metadata = {
  packages: [
    cargoPackage("live", "suprnova-live"),
    cargoPackage("macros", "suprnova-live-macros"),
    cargoPackage("fixture", "suprnova-live-macro-fixture"),
    cargoPackage("support", "suprnova-live-test-support"),
    cargoPackage("shared", "shared"),
    cargoPackage("transitive", "transitive"),
    cargoPackage("cycle-a", "cycle-a"),
    cargoPackage("cycle-b", "cycle-b"),
    cargoPackage("unrelated", "unrelated-workspace-root"),
    cargoPackage("unrelated-dependency", "unrelated-dependency"),
  ],
  workspace_members: ["live", "macros", "fixture", "support", "unrelated"],
  resolve: {
    nodes: [
      resolveNode("live", ["shared"]),
      resolveNode("macros", ["cycle-a"]),
      resolveNode("fixture"),
      resolveNode("support", ["shared"]),
      resolveNode("shared", ["transitive"]),
      resolveNode("transitive"),
      resolveNode("cycle-a", ["cycle-b"]),
      resolveNode("cycle-b", ["cycle-a"]),
      resolveNode("unrelated", ["unrelated-dependency"]),
      resolveNode("unrelated-dependency"),
    ],
  },
};

assert.deepEqual(
  cargoDependencyClosure(metadata, liveRoots)
    .map(({ name }) => name)
    .sort(),
  [
    "cycle-a",
    "cycle-b",
    "shared",
    "suprnova-live",
    "suprnova-live-macro-fixture",
    "suprnova-live-macros",
    "suprnova-live-test-support",
    "transitive",
  ],
  "the shared-workspace inventory follows transitive and cyclic Live edges without including unrelated roots",
);

const missingPackage = structuredClone(metadata);
missingPackage.resolve.nodes
  .find(({ id }) => id === "shared")
  .dependencies.push("missing-package");
assert.throws(
  () => cargoDependencyClosure(missingPackage, liveRoots),
  /dependency package missing-package is absent/u,
  "a reachable dependency missing from Cargo packages fails closed",
);

const missingNode = structuredClone(metadata);
missingNode.packages.push(cargoPackage("missing-node", "missing-node"));
missingNode.resolve.nodes
  .find(({ id }) => id === "shared")
  .dependencies.push("missing-node");
assert.throws(
  () => cargoDependencyClosure(missingNode, liveRoots),
  /resolve node for dependency package missing-node is absent/u,
  "a reachable dependency missing from the resolve graph fails closed",
);

const malformedNode = structuredClone(metadata);
malformedNode.resolve.nodes.find(({ id }) => id === "shared").dependencies =
  null;
assert.throws(
  () => cargoDependencyClosure(malformedNode, liveRoots),
  /invalid dependencies for resolve node shared/u,
  "a malformed resolve node fails closed",
);

const spawnFailure = new Error("spawn rtk ENOENT");
assert.throws(
  () =>
    loadLockedCargoMetadata("/fixture", () => ({
      error: spawnFailure,
      status: null,
      stderr: null,
      stdout: null,
    })),
  (error) =>
    error instanceof Error &&
    error.cause === spawnFailure &&
    error.message.includes("spawn rtk ENOENT"),
  "Cargo metadata spawn failures preserve their root cause",
);

const gitSpawnFailure = new Error("spawn git ENOENT");
assert.throws(
  () =>
    resolveGitWorkspaceRoot("/fixture", () => ({
      error: gitSpawnFailure,
      status: null,
      stderr: null,
      stdout: null,
    })),
  (error) =>
    error instanceof Error &&
    error.cause === gitSpawnFailure &&
    error.message.includes("spawn git ENOENT"),
  "Git workspace spawn failures preserve their root cause",
);

assert.throws(
  () =>
    resolveGitWorkspaceRoot("/fixture", () => ({
      error: undefined,
      status: 128,
      stderr: null,
      stdout: null,
    })),
  /git rev-parse failed in \/fixture with status 128$/u,
  "Git failures tolerate nullable stderr without masking the status",
);

assert.throws(
  () =>
    resolveGitWorkspaceRoot("/fixture", () => ({
      error: undefined,
      status: 0,
      stderr: null,
      stdout: null,
    })),
  /git rev-parse in \/fixture returned no workspace root/u,
  "a successful Git process with nullable stdout fails closed",
);

assert.throws(
  () =>
    loadLockedCargoMetadata("/fixture", () => ({
      error: undefined,
      status: 101,
      stderr: null,
      stdout: "",
    })),
  /cargo metadata failed in \/fixture with status 101$/u,
  "Cargo metadata failures tolerate nullable stderr without masking the status",
);

process.stdout.write("license inventory graph contract ok\n");
