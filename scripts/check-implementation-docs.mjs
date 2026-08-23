#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(scriptDirectory, "..");
const implementationDirectory = path.join(
  repositoryRoot,
  "docs",
  "implementation",
);
const files = [
  "README.md",
  ...fs
    .readdirSync(implementationDirectory)
    .filter((name) => name.endsWith(".md"))
    .sort()
    .map((name) => path.join("docs", "implementation", name)),
];
const failures = [];

for (const relativeFile of files) {
  const fullPath = path.join(repositoryRoot, relativeFile);
  const text = fs.readFileSync(fullPath, "utf8");

  if (text.includes("\r")) {
    failures.push(`${relativeFile}: contains a carriage-return character`);
  }
  if (!text.endsWith("\n") || text.endsWith("\n\n")) {
    failures.push(`${relativeFile}: must end with exactly one newline`);
  }
  if (/^[^\n]*[ \t]+$/m.test(text)) {
    failures.push(`${relativeFile}: contains trailing whitespace`);
  }
  if (/\b(?:TODO|TBD|PLACEHOLDER)\b/u.test(text)) {
    failures.push(`${relativeFile}: contains an unresolved placeholder marker`);
  }
  if (text.includes("-D warnings")) {
    failures.push(
      `${relativeFile}: recommends forbidden blanket warning denial`,
    );
  }

  for (const match of text.matchAll(/!?\[[^\]]*\]\(([^)]+)\)/g)) {
    let target = match[1].trim();
    if (target.startsWith("<") && target.endsWith(">")) {
      target = target.slice(1, -1);
    }
    if (
      target === "" ||
      target.startsWith("#") ||
      target.startsWith("/") ||
      /^[a-z][a-z0-9+.-]*:/i.test(target)
    ) {
      continue;
    }

    let localTarget;
    try {
      localTarget = decodeURIComponent(target.split(/[?#]/, 1)[0]);
    } catch {
      failures.push(`${relativeFile}: malformed relative link: ${target}`);
      continue;
    }
    const resolved = path.resolve(path.dirname(fullPath), localTarget);
    if (!fs.existsSync(resolved)) {
      failures.push(`${relativeFile}: broken relative link: ${target}`);
    }
  }
}

if (failures.length > 0) {
  console.error(
    `implementation-doc-check failed with ${failures.length} issue(s):`,
  );
  for (const failure of failures) {
    console.error(`- ${failure}`);
  }
  process.exit(1);
}

console.log(`implementation-doc-check ok files=${files.length}`);
