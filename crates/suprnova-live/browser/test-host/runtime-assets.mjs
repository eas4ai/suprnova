import { createHash } from "node:crypto";
import { lstat, readFile } from "node:fs/promises";
import { basename, join, resolve } from "node:path";

const MANIFEST_NAME = "suprnova-live.assets.json";
const MANIFEST_FIELDS = Object.freeze([
  "assets",
  "built_at",
  "engine_version",
  "protocol_versions",
  "provenance",
  "runtime_contract_version",
  "schema_version",
  "snapshot_versions",
]);
const ASSET_FIELDS = Object.freeze([
  "bytes",
  "cache_control",
  "capability",
  "capability_version",
  "compatible_core",
  "content_type",
  "file",
  "preload_rel",
  "role",
  "script_kind",
  "sha256",
  "sri",
]);
const EXPECTED_ASSETS = Object.freeze([
  Object.freeze({
    capability: "core@1",
    file: "suprnova-live.classic.js",
    preloadRel: "preload",
    role: "core-classic",
    scriptKind: "classic",
  }),
  Object.freeze({
    capability: "core@1",
    file: "suprnova-live.esm.js",
    preloadRel: "modulepreload",
    role: "core-esm",
    scriptKind: "module",
  }),
  Object.freeze({
    capability: "stimulus@1",
    file: "suprnova-live.stimulus.classic.js",
    preloadRel: "preload",
    role: "stimulus-classic",
    scriptKind: "classic",
  }),
  Object.freeze({
    capability: "stimulus@1",
    file: "suprnova-live.stimulus.esm.js",
    preloadRel: "modulepreload",
    role: "stimulus-esm",
    scriptKind: "module",
  }),
  Object.freeze({
    capability: "uploads@1",
    file: "suprnova-live.uploads.classic.js",
    preloadRel: "preload",
    role: "uploads-classic",
    scriptKind: "classic",
  }),
  Object.freeze({
    capability: "uploads@1",
    file: "suprnova-live.uploads.esm.js",
    preloadRel: "modulepreload",
    role: "uploads-esm",
    scriptKind: "module",
  }),
  Object.freeze({
    capability: "async@1",
    file: "suprnova-live.async.classic.js",
    preloadRel: "preload",
    role: "async-classic",
    scriptKind: "classic",
  }),
  Object.freeze({
    capability: "async@1",
    file: "suprnova-live.async.esm.js",
    preloadRel: "modulepreload",
    role: "async-esm",
    scriptKind: "module",
  }),
]);
const SHA256 = /^[0-9a-f]{64}$/u;

function fail(reason) {
  throw new Error(reason);
}

function exactKeys(value, expected) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) return false;
  const actual = Object.keys(value).sort();
  return actual.length === expected.length && actual.every((key, index) => key === expected[index]);
}

function exactArray(value, expected) {
  return (
    Array.isArray(value) &&
    value.length === expected.length &&
    value.every((entry, index) => entry === expected[index])
  );
}

function exactProvenance(value) {
  if (!exactKeys(value, ["idiomorph"])) return false;
  const idiomorph = value.idiomorph;
  return (
    exactKeys(idiomorph, ["bundled", "license", "name", "version"]) &&
    idiomorph.bundled === true &&
    idiomorph.license === "0BSD" &&
    idiomorph.name === "idiomorph" &&
    idiomorph.version === "0.7.4"
  );
}

async function regularFile(path, reason) {
  try {
    const metadata = await lstat(path);
    if (!metadata.isFile() || metadata.isSymbolicLink()) fail(reason);
    return await readFile(path);
  } catch (error) {
    if (error instanceof Error && error.message === reason) throw error;
    fail(reason);
  }
}

function validateManifest(manifest) {
  if (
    !exactKeys(manifest, MANIFEST_FIELDS) ||
    manifest.schema_version !== 2 ||
    manifest.engine_version !== "0.1.0" ||
    manifest.runtime_contract_version !== 1 ||
    !exactArray(manifest.protocol_versions, [1, 2]) ||
    !exactArray(manifest.snapshot_versions, [1]) ||
    manifest.built_at !== "1970-01-01T00:00:00.000Z" ||
    !exactProvenance(manifest.provenance) ||
    !Array.isArray(manifest.assets) ||
    manifest.assets.length !== EXPECTED_ASSETS.length
  ) {
    fail("runtime_asset_manifest_invalid");
  }
}

function validateAssetRecord(asset, expected) {
  if (
    !exactKeys(asset, ASSET_FIELDS) ||
    typeof asset.file !== "string" ||
    basename(asset.file) !== asset.file ||
    asset.file !== expected.file ||
    asset.role !== expected.role ||
    asset.capability !== expected.capability ||
    asset.capability_version !== 1 ||
    asset.compatible_core !== ">=0.1.0 <0.2.0" ||
    asset.content_type !== "text/javascript; charset=utf-8" ||
    asset.script_kind !== expected.scriptKind ||
    asset.preload_rel !== expected.preloadRel ||
    asset.cache_control !== "public, max-age=31536000, immutable" ||
    !Number.isSafeInteger(asset.bytes) ||
    asset.bytes < 0 ||
    typeof asset.sha256 !== "string" ||
    !SHA256.test(asset.sha256) ||
    typeof asset.sri !== "string"
  ) {
    fail("runtime_asset_manifest_invalid");
  }
}

function immutableSnapshot(artifactRoot, manifestBytes, entries) {
  const snapshot = new Map(entries);
  snapshot.set(
    MANIFEST_NAME,
    Object.freeze({
      bytes: Buffer.from(manifestBytes),
      cacheControl: "public, max-age=31536000, immutable",
      contentType: "application/json; charset=utf-8",
    }),
  );
  return Object.freeze({
    artifactRoot,
    asset(file) {
      const selected = snapshot.get(file);
      if (selected === undefined) return null;
      return Object.freeze({
        bytes: Buffer.from(selected.bytes),
        cacheControl: selected.cacheControl,
        contentType: selected.contentType,
      });
    },
  });
}

export async function validateRuntimeAssets(root) {
  const artifactRoot = resolve(root);
  const manifestBytes = await regularFile(
    join(artifactRoot, MANIFEST_NAME),
    "runtime_asset_manifest_unavailable",
  );
  let manifest;
  try {
    manifest = JSON.parse(manifestBytes.toString("utf8"));
  } catch {
    fail("runtime_asset_manifest_invalid");
  }
  validateManifest(manifest);

  const entries = await Promise.all(
    manifest.assets.map(async (asset, index) => {
      const expected = EXPECTED_ASSETS[index];
      if (expected === undefined) fail("runtime_asset_manifest_invalid");
      validateAssetRecord(asset, expected);
      const content = await regularFile(
        join(artifactRoot, expected.file),
        `runtime_asset_unavailable:${expected.file}`,
      );
      const digest = createHash("sha256").update(content).digest();
      if (
        content.byteLength !== asset.bytes ||
        digest.toString("hex") !== asset.sha256 ||
        `sha256-${digest.toString("base64")}` !== asset.sri
      ) {
        fail(`runtime_asset_mismatch:${expected.file}`);
      }
      return [
        expected.file,
        Object.freeze({
          bytes: Buffer.from(content),
          cacheControl: asset.cache_control,
          contentType: asset.content_type,
        }),
      ];
    }),
  );
  return immutableSnapshot(artifactRoot, manifestBytes, entries);
}

export async function afterRuntimeAssetsValidated(root, start) {
  const artifacts = await validateRuntimeAssets(root);
  return start(artifacts);
}
