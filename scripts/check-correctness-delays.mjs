#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const allowedCategories = new Set(["fake-clock", "product-timer", "watchdog"]);
const allowPrefix = "suprnova-correctness-delay-allow:";

function isIdentifierStart(character) {
  return /[A-Za-z_$]/u.test(character ?? "");
}

function isIdentifierPart(character) {
  return /[A-Za-z0-9_$]/u.test(character ?? "");
}

function tokenizeJavaScript(source) {
  const tokens = [];
  const comments = [];
  const regularExpressionPrefixes = new Set([
    "(",
    "[",
    "{",
    ",",
    ";",
    ":",
    "=",
    "!",
    "?",
    "&",
    "|",
    "case",
    "delete",
    "do",
    "else",
    "in",
    "instanceof",
    "new",
    "of",
    "return",
    "throw",
    "typeof",
    "void",
    "yield",
  ]);
  let index = 0;
  let line = 1;

  function canStartRegularExpression() {
    const previous = tokens.at(-1)?.value;
    return previous === undefined || regularExpressionPrefixes.has(previous);
  }

  function skipRegularExpression() {
    index += 1;
    let characterClass = false;
    while (index < source.length && source[index] !== "\n") {
      if (source[index] === "\\") {
        index += 2;
      } else if (source[index] === "[") {
        characterClass = true;
        index += 1;
      } else if (source[index] === "]") {
        characterClass = false;
        index += 1;
      } else if (source[index] === "/" && !characterClass) {
        index += 1;
        while (/[A-Za-z]/u.test(source[index] ?? "")) index += 1;
        return;
      } else {
        index += 1;
      }
    }
  }

  function skipQuoted(quote) {
    index += 1;
    while (index < source.length) {
      if (source[index] === "\\") {
        index += 2;
      } else if (source[index] === quote) {
        index += 1;
        return;
      } else {
        if (source[index] === "\n") line += 1;
        index += 1;
      }
    }
  }

  function scanCode(stopAtTemplateBrace = false) {
    let braceDepth = 0;
    while (index < source.length) {
      const character = source[index];
      const next = source[index + 1];

      if (character === "\n") {
        line += 1;
        index += 1;
        continue;
      }
      if (/\s/u.test(character)) {
        index += 1;
        continue;
      }
      if (character === "/" && next === "/") {
        const commentLine = line;
        index += 2;
        const start = index;
        while (index < source.length && source[index] !== "\n") index += 1;
        comments.push({ line: commentLine, text: source.slice(start, index) });
        continue;
      }
      if (character === "/" && next === "*") {
        const commentLine = line;
        index += 2;
        const start = index;
        while (
          index < source.length &&
          !(source[index] === "*" && source[index + 1] === "/")
        ) {
          if (source[index] === "\n") line += 1;
          index += 1;
        }
        comments.push({ line: commentLine, text: source.slice(start, index) });
        if (index < source.length) index += 2;
        continue;
      }
      if (character === "/" && canStartRegularExpression()) {
        skipRegularExpression();
        continue;
      }
      if (character === '"' || character === "'") {
        skipQuoted(character);
        continue;
      }
      if (character === "`") {
        index += 1;
        while (index < source.length) {
          if (source[index] === "\\") {
            index += 2;
          } else if (source[index] === "`") {
            index += 1;
            break;
          } else if (source[index] === "$" && source[index + 1] === "{") {
            index += 2;
            scanCode(true);
          } else {
            if (source[index] === "\n") line += 1;
            index += 1;
          }
        }
        continue;
      }
      if (stopAtTemplateBrace && character === "}" && braceDepth === 0) {
        index += 1;
        return;
      }
      if (character === "{") braceDepth += 1;
      if (character === "}" && braceDepth > 0) braceDepth -= 1;
      if (isIdentifierStart(character)) {
        const tokenLine = line;
        const start = index;
        index += 1;
        while (isIdentifierPart(source[index])) index += 1;
        tokens.push({ line: tokenLine, value: source.slice(start, index) });
        continue;
      }
      tokens.push({ line, value: character });
      index += 1;
    }
  }

  scanCode();
  return { comments, tokens };
}

function tokenizeRust(source) {
  const tokens = [];
  const comments = [];
  let index = 0;
  let line = 1;

  function skipQuoted(quote) {
    index += 1;
    while (index < source.length) {
      if (source[index] === "\\") {
        index += 2;
      } else if (source[index] === quote) {
        index += 1;
        return;
      } else {
        if (source[index] === "\n") line += 1;
        index += 1;
      }
    }
  }

  while (index < source.length) {
    const character = source[index];
    const next = source[index + 1];
    if (character === "\n") {
      line += 1;
      index += 1;
      continue;
    }
    if (/\s/u.test(character)) {
      index += 1;
      continue;
    }
    if (character === "/" && next === "/") {
      const commentLine = line;
      index += 2;
      const start = index;
      while (index < source.length && source[index] !== "\n") index += 1;
      comments.push({ line: commentLine, text: source.slice(start, index) });
      continue;
    }
    if (character === "/" && next === "*") {
      const commentLine = line;
      index += 2;
      const start = index;
      let depth = 1;
      while (index < source.length && depth > 0) {
        if (source[index] === "/" && source[index + 1] === "*") {
          depth += 1;
          index += 2;
        } else if (source[index] === "*" && source[index + 1] === "/") {
          depth -= 1;
          index += 2;
        } else {
          if (source[index] === "\n") line += 1;
          index += 1;
        }
      }
      comments.push({
        line: commentLine,
        text: source.slice(start, index - 2),
      });
      continue;
    }
    const rawPrefix = source.slice(index).match(/^r(#+)?"/u);
    if (rawPrefix) {
      const hashes = rawPrefix[1] ?? "";
      index += rawPrefix[0].length;
      const terminator = `"${hashes}`;
      const end = source.indexOf(terminator, index);
      const stop = end === -1 ? source.length : end + terminator.length;
      line += source.slice(index, stop).split("\n").length - 1;
      index = stop;
      continue;
    }
    if (character === '"') {
      skipQuoted(character);
      continue;
    }
    if (character === "'") {
      let end = index + 1;
      while (end < source.length && source[end] !== "\n") {
        if (source[end] === "\\") end += 2;
        else if (source[end] === "'") break;
        else end += 1;
      }
      if (source[end] === "'") {
        index = end + 1;
        continue;
      }
    }
    if (isIdentifierStart(character)) {
      const tokenLine = line;
      const start = index;
      index += 1;
      while (isIdentifierPart(source[index])) index += 1;
      tokens.push({ line: tokenLine, value: source.slice(start, index) });
      continue;
    }
    if (character === ":" && next === ":") {
      tokens.push({ line, value: "::" });
      index += 2;
      continue;
    }
    tokens.push({ line, value: character });
    index += 1;
  }
  return { comments, tokens };
}

function tokenSequenceAt(tokens, index, values) {
  return values.every(
    (value, offset) => tokens[index + offset]?.value === value,
  );
}

function javascriptViolations(tokens) {
  const violations = [];
  for (let index = 0; index < tokens.length; index += 1) {
    if (
      tokens[index - 1]?.value === "." &&
      tokens[index].value === "waitForTimeout" &&
      tokens[index + 1]?.value === "("
    ) {
      violations.push({ kind: "playwright-timeout", line: tokens[index].line });
    }
    if (!tokenSequenceAt(tokens, index, ["new", "Promise", "("])) continue;
    let depth = 0;
    let hasTimeout = false;
    for (let cursor = index + 2; cursor < tokens.length; cursor += 1) {
      if (tokens[cursor].value === "(") depth += 1;
      if (tokens[cursor].value === ")") {
        depth -= 1;
        if (depth === 0) break;
      }
      if (
        tokens[cursor].value === "setTimeout" &&
        tokens[cursor + 1]?.value === "("
      ) {
        hasTimeout = true;
      }
    }
    if (hasTimeout) {
      violations.push({ kind: "promise-timeout", line: tokens[index].line });
    }
  }
  return violations;
}

function rustViolations(tokens) {
  const violations = [];
  const patterns = [
    { kind: "rust-sleep", values: ["tokio", "::", "time", "::", "sleep", "("] },
    {
      kind: "rust-sleep",
      values: ["tokio", "::", "time", "::", "sleep_until", "("],
    },
    { kind: "rust-sleep", values: ["std", "::", "thread", "::", "sleep", "("] },
    {
      kind: "rust-sleep",
      unqualified: true,
      values: ["thread", "::", "sleep", "("],
    },
    {
      kind: "rust-sleep",
      values: ["std", "::", "thread", "::", "park_timeout", "("],
    },
    {
      kind: "rust-spin-wait",
      values: ["std", "::", "thread", "::", "yield_now", "("],
    },
    {
      kind: "rust-spin-wait",
      unqualified: true,
      values: ["thread", "::", "yield_now", "("],
    },
    {
      kind: "rust-spin-wait",
      values: ["tokio", "::", "task", "::", "yield_now", "("],
    },
    {
      kind: "rust-spin-wait",
      values: ["std", "::", "hint", "::", "spin_loop", "("],
    },
    {
      kind: "rust-spin-wait",
      values: ["core", "::", "hint", "::", "spin_loop", "("],
    },
  ];
  for (let index = 0; index < tokens.length; index += 1) {
    for (const pattern of patterns) {
      if (
        (!pattern.unqualified || tokens[index - 1]?.value !== "::") &&
        tokenSequenceAt(tokens, index, pattern.values)
      ) {
        violations.push({ kind: pattern.kind, line: tokens[index].line });
        break;
      }
    }
  }
  return violations;
}

function applyAllowPolicy(comments, violations) {
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

export function scanSource({ filePath, language, source }) {
  const tokenized =
    language === "rust" ? tokenizeRust(source) : tokenizeJavaScript(source);
  const violations =
    language === "rust"
      ? rustViolations(tokenized.tokens)
      : javascriptViolations(tokenized.tokens);
  return applyAllowPolicy(tokenized.comments, violations).map((failure) => ({
    ...failure,
    filePath,
  }));
}

function matchingFiles(directory, expression) {
  if (!fs.existsSync(directory)) return [];
  return fs
    .readdirSync(directory, { withFileTypes: true })
    .filter((entry) => entry.isFile() && expression.test(entry.name))
    .map((entry) => path.join(directory, entry.name));
}

function verificationSurfaces(repositoryRoot) {
  return [
    path.join(repositoryRoot, "src/upload/provider.rs"),
    ...matchingFiles(
      path.join(repositoryRoot, "tests"),
      /^iteration_004.*\.rs$/u,
    ),
    path.join(
      repositoryRoot,
      "crates/suprnova-live-test-support/tests/reference_host.rs",
    ),
    ...matchingFiles(
      path.join(repositoryRoot, "browser/tests"),
      /^async-.*\.test\.ts$/u,
    ),
    ...matchingFiles(
      path.join(repositoryRoot, "browser/tests"),
      /^upload-.*\.test\.ts$/u,
    ),
    path.join(repositoryRoot, "browser/tests/bounded-resources.test.ts"),
    ...matchingFiles(
      path.join(repositoryRoot, "browser/e2e"),
      /^iteration-004.*\.spec\.ts$/u,
    ),
    path.join(repositoryRoot, "browser/e2e/async-lifecycle.spec.ts"),
    path.join(repositoryRoot, "browser/e2e/uploads.spec.ts"),
    path.join(repositoryRoot, "browser/test-host/async-lifecycle.mjs"),
    path.join(repositoryRoot, "browser/test-host/iteration-004.mjs"),
  ];
}

export function scanRepository(repositoryRoot) {
  const failures = [];
  for (const filePath of verificationSurfaces(repositoryRoot)) {
    if (!fs.existsSync(filePath)) {
      failures.push({
        filePath: path.relative(repositoryRoot, filePath),
        kind: "missing-surface",
        line: 1,
      });
      continue;
    }
    failures.push(
      ...scanSource({
        filePath: path.relative(repositoryRoot, filePath),
        language: filePath.endsWith(".rs") ? "rust" : "javascript",
        source: fs.readFileSync(filePath, "utf8"),
      }),
    );
  }
  return failures;
}

function main() {
  const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
  const repositoryRoot = path.resolve(scriptDirectory, "..");
  const failures = scanRepository(repositoryRoot);
  if (failures.length > 0) {
    for (const failure of failures) {
      process.stderr.write(
        `correctness-delay scanner: ${failure.filePath}:${failure.line}: ${failure.kind}\n`,
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
