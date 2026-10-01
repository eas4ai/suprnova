#!/usr/bin/env node

import assert from "node:assert/strict";

import {
  cargoDependencyClosure,
  loadLockedCargoMetadata,
  resolveGitWorkspaceRoot,
  thirdPartyCargoDependencyClosure,
} from "../scripts/license-inventory-cargo.mjs";

const liveRoots = [
  "suprnova-live",
  "suprnova-macros",
  "suprnova-live-macro-fixture",
  "suprnova-live-test-support",
];

function cargoPackage(id, name, source = "registry+test") {
  return {
    id,
    name,
    version: "1.0.0",
    license: "MIT",
    source,
  };
}

function resolveNode(id, dependencies = []) {
  return { id, dependencies };
}

const metadata = {
  packages: [
    cargoPackage("live", "suprnova-live"),
    cargoPackage("macros", "suprnova-macros"),
    cargoPackage("fixture", "suprnova-live-macro-fixture"),
    cargoPackage("support", "suprnova-live-test-support"),
    cargoPackage("framework", "suprnova"),
    cargoPackage("framework-external", "framework-external"),
    cargoPackage("first-party-path", "first-party-path", null),
    cargoPackage("path-external", "path-external"),
    cargoPackage("vendored-path", "vendored-path", null),
    cargoPackage("shared", "shared"),
    cargoPackage("transitive", "transitive"),
    cargoPackage("cycle-a", "cycle-a"),
    cargoPackage("cycle-b", "cycle-b"),
    cargoPackage("unrelated", "unrelated-workspace-root"),
    cargoPackage("unrelated-dependency", "unrelated-dependency"),
  ],
  workspace_members: [
    "live",
    "macros",
    "fixture",
    "support",
    "framework",
    "unrelated",
  ],
  resolve: {
    nodes: [
      resolveNode("live", ["shared"]),
      resolveNode("macros", ["cycle-a", "framework"]),
      resolveNode("fixture", ["first-party-path", "vendored-path"]),
      resolveNode("support", ["shared"]),
      resolveNode("framework", ["framework-external"]),
      resolveNode("framework-external"),
      resolveNode("first-party-path", ["path-external"]),
      resolveNode("path-external"),
      resolveNode("vendored-path"),
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
    "first-party-path",
    "framework-external",
    "path-external",
    "shared",
    "suprnova",
    "suprnova-live",
    "suprnova-live-macro-fixture",
    "suprnova-live-test-support",
    "suprnova-macros",
    "transitive",
    "vendored-path",
  ],
  "the shared-workspace inventory follows transitive and cyclic Live edges without including unrelated roots",
);

assert.deepEqual(
  thirdPartyCargoDependencyClosure(metadata, liveRoots, ["first-party-path"])
    .map(({ name }) => name)
    .sort(),
  [
    "cycle-a",
    "cycle-b",
    "framework-external",
    "path-external",
    "shared",
    "transitive",
    "vendored-path",
  ],
  "third-party inventory excludes explicitly proven first-party IDs while retaining unclassified path dependencies and their external closure",
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
