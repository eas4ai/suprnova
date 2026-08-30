import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { build } from "esbuild";

const browserRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const entryPoint = resolve(browserRoot, "tests/support/iteration-004-conformance.ts");
const outputPath = resolve(browserRoot, "generated/iteration-004-conformance.mjs");
const check = process.argv.includes("--check");

const result = await build({
  banner: { js: "// @generated from iteration-004-conformance.ts; do not edit." },
  bundle: true,
  charset: "utf8",
  entryPoints: [entryPoint],
  format: "esm",
  legalComments: "none",
  minify: false,
  platform: "node",
  sourcemap: false,
  target: "node20",
  treeShaking: true,
  write: false,
});
const output = result.outputFiles[0];
if (output === undefined) throw new Error("iteration_004_conformance_bundle_missing");

if (check) {
  const existing = await readFile(outputPath);
  if (!existing.equals(output.contents)) {
    throw new Error("iteration_004_conformance_bundle_drift");
  }
} else {
  await mkdir(dirname(outputPath), { recursive: true });
  await writeFile(outputPath, output.contents);
}
