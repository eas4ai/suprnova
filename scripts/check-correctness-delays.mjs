#!/usr/bin/env node

import fs from "node:fs";
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import {
  loadTypeScript,
  scanJavaScript,
} from "./correctness-delay-javascript.mjs";
import { iteration004VerificationSurfaces } from "./iteration-004-verification-surfaces.mjs";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const defaultRepositoryRoot = path.resolve(scriptDirectory, "..");
const allowedCategories = new Set(["fake-clock", "product-timer", "watchdog"]);
const allowPrefix = "suprnova-correctness-delay-allow:";

function allowPolicy(comments, violations) {
  const failures = [];
  const allows = [];
  for (const comment of comments) {
    const text = comment.text.trim();
    if (!text.startsWith(allowPrefix)) continue;
    const match = text.match(
      /^suprnova-correctness-delay-allow:\s*([a-z-]+)\s+--\s+(.+)$/u,
    );
    if (
      !match ||
      !allowedCategories.has(match[1]) ||
      match[2].trim().length < 12
    ) {
      failures.push({ kind: "invalid-allow", line: comment.line });
      continue;
    }
    allows.push({ line: comment.line, used: false });
  }
  const remaining = [];
  for (const violation of violations) {
    const allow = allows.find(
      (candidate) => !candidate.used && candidate.line + 1 === violation.line,
    );
    if (allow) allow.used = true;
    else remaining.push(violation);
  }
  for (const allow of allows) {
    if (!allow.used) failures.push({ kind: "unused-allow", line: allow.line });
  }
  return [...failures, ...remaining].sort(
    (left, right) =>
      left.line - right.line || left.kind.localeCompare(right.kind),
  );
}

export function scanSource({
  filePath,
  source,
  repositoryRoot = defaultRepositoryRoot,
}) {
  const scanned = scanJavaScript(
    loadTypeScript(repositoryRoot),
    filePath,
    source,
  );
  return allowPolicy(scanned.comments, scanned.violations).map((failure) => ({
    ...failure,
    filePath,
  }));
}

export function parseRustCandidates(repositoryRoot, rustCandidates) {
  const executableName =
    process.platform === "win32"
      ? "correctness-delay-rust-parser.exe"
      : "correctness-delay-rust-parser";
  const parserPath = path.join(
    process.env.CARGO_TARGET_DIR ?? path.join(repositoryRoot, "target"),
    "debug",
    executableName,
  );
  if (!fs.existsSync(parserPath)) {
    return [{ filePath: parserPath, kind: "rust-parser-unavailable", line: 1 }];
  }
  const parsed = spawnSync(parserPath, [], {
    encoding: "utf8",
    input: JSON.stringify(rustCandidates),
    maxBuffer: 16 * 1024 * 1024,
  });
  if (parsed.status !== 0 || parsed.error !== undefined) {
    return [{ filePath: parserPath, kind: "rust-parser-failed", line: 1 }];
  }
  try {
    return JSON.parse(parsed.stdout);
  } catch {
    return [
      { filePath: parserPath, kind: "rust-parser-invalid-output", line: 1 },
    ];
  }
}

export function scanRepository(repositoryRoot) {
  const failures = [];
  const rustCandidates = [];
  for (const surface of iteration004VerificationSurfaces(repositoryRoot)) {
    const relative = path.relative(repositoryRoot, surface.filePath);
    if (!fs.existsSync(surface.filePath)) {
      failures.push({ filePath: relative, kind: "missing-surface", line: 1 });
      continue;
    }
    const source = fs.readFileSync(surface.filePath, "utf8");
    const rust = surface.filePath.endsWith(".rs");
    if (rust) {
      rustCandidates.push({ file_path: relative, source });
      continue;
    }
    failures.push(
      ...scanSource({
        filePath: relative,
        repositoryRoot,
        source,
      }),
    );
  }
  failures.push(...parseRustCandidates(repositoryRoot, rustCandidates));
  return failures;
}

function main() {
  const failures = scanRepository(defaultRepositoryRoot);
  if (failures.length > 0) {
    for (const failure of failures) {
      process.stderr.write(
        `correctness-delay scanner: ${failure.filePath}:${String(failure.line)}: ${failure.kind}\n`,
      );
    }
    process.exitCode = 1;
    return;
  }
  process.stdout.write("correctness-delay scanner ok\n");
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href
) {
  main();
}
