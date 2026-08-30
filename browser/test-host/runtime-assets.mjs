import { createHash } from "node:crypto";
import { lstat, readFile } from "node:fs/promises";
import { basename, join, resolve } from "node:path";

const MANIFEST_NAME = "suprnova-live.assets.json";
const SHA256 = /^[0-9a-f]{64}$/u;

function fail(reason) {
  throw new Error(reason);
}

async function regularFile(path, reason) {
  let metadata;
  try {
    metadata = await lstat(path);
  } catch {
    fail(reason);
  }
  if (!metadata.isFile() || metadata.isSymbolicLink()) fail(reason);
  return readFile(path);
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
  if (manifest?.schema_version !== 2 || !Array.isArray(manifest.assets)) {
    fail("runtime_asset_manifest_invalid");
  }

  const names = new Set();
  await Promise.all(
    manifest.assets.map(async (asset) => {
      const file = asset?.file;
      if (
        typeof file !== "string" ||
        file.length === 0 ||
        basename(file) !== file ||
        names.has(file) ||
        !Number.isSafeInteger(asset?.bytes) ||
        asset.bytes < 0 ||
        typeof asset?.sha256 !== "string" ||
        !SHA256.test(asset.sha256)
      ) {
        fail("runtime_asset_manifest_invalid");
      }
      names.add(file);
      const content = await regularFile(
        join(artifactRoot, file),
        `runtime_asset_unavailable:${file}`,
      );
      if (
        content.byteLength !== asset.bytes ||
        createHash("sha256").update(content).digest("hex") !== asset.sha256
      ) {
        fail(`runtime_asset_mismatch:${file}`);
      }
    }),
  );
  if (names.size === 0) fail("runtime_asset_manifest_invalid");
  return Object.freeze({ artifactRoot, manifest: Object.freeze(manifest) });
}

export async function afterRuntimeAssetsValidated(root, start) {
  const artifacts = await validateRuntimeAssets(root);
  return start(artifacts);
}
