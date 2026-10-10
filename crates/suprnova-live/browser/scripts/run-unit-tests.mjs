import { spawnSync } from "node:child_process";
import { lstatSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const browserRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

function selectedPath(argument) {
  if (argument.startsWith("-") || !argument.endsWith(".test.ts")) return null;
  return resolve(browserRoot, argument.includes("/") ? argument : `tests/${argument}`);
}

export function validateRequestedTestFiles(arguments_) {
  for (const argument of arguments_) {
    const path = selectedPath(argument);
    if (path === null) continue;
    let metadata;
    try {
      metadata = lstatSync(path);
    } catch {
      throw new Error(`named_test_file_missing:${argument}`);
    }
    if (!metadata.isFile() || metadata.isSymbolicLink()) {
      throw new Error(`named_test_file_invalid:${argument}`);
    }
  }
}

function main() {
  const arguments_ = process.argv.slice(2);
  try {
    validateRequestedTestFiles(arguments_);
  } catch (error) {
    process.stderr.write(`${error instanceof Error ? error.message : "test_selection_invalid"}\n`);
    process.exitCode = 64;
    return;
  }
  const execution = spawnSync(
    process.execPath,
    [resolve(browserRoot, "node_modules/vitest/vitest.mjs"), "run", ...arguments_],
    { cwd: browserRoot, stdio: "inherit" },
  );
  if (execution.error !== undefined) throw execution.error;
  process.exitCode = execution.status ?? 1;
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main();
}
