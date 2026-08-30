import { mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { buildRuntimeAssets } from "./build.mjs";

const EXPECTED_OUTPUTS = Object.freeze([
  "index.d.ts",
  "suprnova-live.assets.json",
  "suprnova-live.async.classic.js",
  "suprnova-live.async.esm.js",
  "suprnova-live.classic.js",
  "suprnova-live.esm.js",
  "suprnova-live.stimulus.classic.js",
  "suprnova-live.stimulus.esm.js",
  "suprnova-live.uploads.classic.js",
  "suprnova-live.uploads.esm.js",
]);

const root = await mkdtemp(join(tmpdir(), "suprnova-live-build-check-"));
try {
  const first = join(root, "first");
  const second = join(root, "second");
  await buildRuntimeAssets(first);
  await buildRuntimeAssets(second);
  const firstNames = (await readdir(first)).sort();
  const secondNames = (await readdir(second)).sort();
  if (JSON.stringify(firstNames) !== JSON.stringify(EXPECTED_OUTPUTS)) {
    throw new Error("build_output_set_incomplete");
  }
  if (JSON.stringify(firstNames) !== JSON.stringify(secondNames)) {
    throw new Error("build_output_set_changed");
  }
  for (const name of firstNames) {
    const [left, right] = await Promise.all([
      readFile(join(first, name)),
      readFile(join(second, name)),
    ]);
    if (!left.equals(right)) throw new Error(`build_output_changed:${name}`);
  }
  process.stdout.write(`reproducible build: ${firstNames.join(", ")}\n`);
} finally {
  await rm(root, { recursive: true, force: true });
}
