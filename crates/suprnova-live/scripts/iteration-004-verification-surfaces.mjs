import fs from "node:fs";
import path from "node:path";

const rustSource = /\.rs$/u;
const browserSource = /\.(?:cjs|cts|js|jsx|mjs|mts|ts|tsx)$/u;
const generatedDirectories = new Set([".git", "node_modules", "target"]);

function filesBelow(directory, expression) {
  if (!fs.existsSync(directory)) return [];
  return fs
    .readdirSync(directory, { recursive: true, withFileTypes: true })
    .filter((entry) => {
      if (!entry.isFile() || !expression.test(entry.name)) return false;
      const relative = path.relative(
        directory,
        path.join(entry.parentPath, entry.name),
      );
      return !relative
        .split(path.sep)
        .some((segment) => generatedDirectories.has(segment));
    })
    .map((entry) => path.join(entry.parentPath, entry.name));
}

/**
 * Returns the complete recursively owned Iteration 004 verification surface.
 *
 * Every Rust source is compiler-checked so inline tests, separate test modules,
 * benches, fuzz targets, and harness code cannot escape through naming. Every
 * repository and browser verification tree is recursively enumerated rather
 * than maintained as a filename allowlist.
 */
export function iteration004VerificationSurfaces(repositoryRoot) {
  const rustRoots = [
    "src",
    "tests",
    "benches",
    "fuzz/fuzz_targets",
    "crates/suprnova-live-test-support/src",
    "crates/suprnova-live-test-support/tests",
  ];
  const browserRoots = [
    "tests",
    "browser/tests",
    "browser/e2e",
    "browser/test-host",
  ];
  const files = new Set();
  for (const root of rustRoots) {
    for (const filePath of filesBelow(
      path.join(repositoryRoot, root),
      rustSource,
    )) {
      files.add(filePath);
    }
  }
  for (const root of browserRoots) {
    for (const filePath of filesBelow(
      path.join(repositoryRoot, root),
      browserSource,
    )) {
      files.add(filePath);
    }
  }
  return [...files]
    .sort((left, right) => left.localeCompare(right))
    .map((filePath) => ({ filePath, region: "full" }));
}
