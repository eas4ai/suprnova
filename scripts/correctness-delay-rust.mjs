function identifierStart(character) {
  return /[A-Za-z_]/u.test(character ?? "");
}

function identifierPart(character) {
  return /[A-Za-z0-9_]/u.test(character ?? "");
}

export function tokenizeRust(source) {
  const tokens = [];
  const comments = [];
  const errors = [];
  const delimiters = [];
  let index = 0;
  let line = 1;

  const push = (value, start, end, tokenLine) =>
    tokens.push({ end, line: tokenLine, start, value });
  const closeDelimiter = (value, tokenLine) => {
    const expected = value === ")" ? "(" : value === "]" ? "[" : "{";
    if (delimiters.pop()?.value !== expected)
      errors.push({ kind: "parse-error", line: tokenLine });
  };

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
      if (depth !== 0) errors.push({ kind: "parse-error", line: commentLine });
      comments.push({
        line: commentLine,
        text: source.slice(start, Math.max(start, index - 2)),
      });
      continue;
    }
    const rawPrefix = source.slice(index).match(/^(?:br|r)(#+)?"/u);
    if (rawPrefix) {
      const rawLine = line;
      const hashes = rawPrefix[1] ?? "";
      index += rawPrefix[0].length;
      const terminator = `"${hashes}`;
      const end = source.indexOf(terminator, index);
      if (end === -1) {
        errors.push({ kind: "parse-error", line: rawLine });
        line += source.slice(index).split("\n").length - 1;
        index = source.length;
      } else {
        line +=
          source.slice(index, end + terminator.length).split("\n").length - 1;
        index = end + terminator.length;
      }
      continue;
    }
    if (character === '"' || (character === "b" && next === '"')) {
      const quoteLine = line;
      if (character === "b") index += 1;
      index += 1;
      let closed = false;
      while (index < source.length) {
        if (source[index] === "\\") index += 2;
        else if (source[index] === '"') {
          index += 1;
          closed = true;
          break;
        } else {
          if (source[index] === "\n") line += 1;
          index += 1;
        }
      }
      if (!closed) errors.push({ kind: "parse-error", line: quoteLine });
      continue;
    }
    if (character === "'") {
      const tokenLine = line;
      let end = index + 1;
      if (source[end] === "\\") end += 2;
      else end += 1;
      if (source[end] === "'") {
        index = end + 1;
        continue;
      }
      push("'", index, index + 1, tokenLine);
      index += 1;
      continue;
    }
    if (identifierStart(character)) {
      const tokenLine = line;
      const start = index;
      index += 1;
      while (identifierPart(source[index])) index += 1;
      push(source.slice(start, index), start, index, tokenLine);
      continue;
    }
    const tokenLine = line;
    const start = index;
    if (character === ":" && next === ":") {
      index += 2;
      push("::", start, index, tokenLine);
      continue;
    }
    index += 1;
    push(character, start, index, tokenLine);
    if (character === "(" || character === "[" || character === "{") {
      delimiters.push({ line: tokenLine, value: character });
    } else if (character === ")" || character === "]" || character === "}") {
      closeDelimiter(character, tokenLine);
    }
  }
  for (const delimiter of delimiters)
    errors.push({ kind: "parse-error", line: delimiter.line });
  return { comments, errors, tokens };
}

function parseUseItem(tokens, state, prefix, aliases, stopValues) {
  if (tokens[state.index]?.value === "{") {
    state.index += 1;
    while (state.index < tokens.length && tokens[state.index].value !== "}") {
      if (!parseUseItem(tokens, state, prefix, aliases, new Set([",", "}"])))
        return false;
      if (tokens[state.index]?.value === ",") state.index += 1;
    }
    if (tokens[state.index]?.value !== "}") return false;
    state.index += 1;
    return true;
  }

  const segments = [];
  while (state.index < tokens.length) {
    const token = tokens[state.index]?.value;
    if (!identifierStart(token?.[0]) && token !== "self") break;
    segments.push(token);
    state.index += 1;
    if (tokens[state.index]?.value !== "::") break;
    state.index += 1;
    if (tokens[state.index]?.value === "*") {
      state.index += 1;
      return stopValues.has(tokens[state.index]?.value);
    }
    if (tokens[state.index]?.value === "{") {
      const groupPrefix = [
        ...prefix,
        ...segments.filter((segment) => segment !== "self"),
      ];
      return parseUseItem(tokens, state, groupPrefix, aliases, stopValues);
    }
  }
  if (segments.length === 0) return false;
  const canonical = [
    ...prefix,
    ...segments.filter((segment) => segment !== "self"),
  ];
  let local = segments.at(-1);
  if (tokens[state.index]?.value === "as") {
    state.index += 1;
    local = tokens[state.index]?.value;
    if (!identifierStart(local?.[0])) return false;
    state.index += 1;
  }
  if (!stopValues.has(tokens[state.index]?.value)) return false;
  if (local !== "self" && local !== "_") aliases.set(local, canonical);
  return true;
}

function aliasesFrom(tokens, errors) {
  const aliases = new Map([["thread", ["std", "thread"]]]);
  for (let index = 0; index < tokens.length; index += 1) {
    if (tokens[index].value !== "use") continue;
    const state = { index: index + 1 };
    if (
      !parseUseItem(tokens, state, [], aliases, new Set([";"])) ||
      tokens[state.index]?.value !== ";"
    ) {
      errors.push({ kind: "parse-error", line: tokens[index].line });
    } else {
      index = state.index;
    }
  }
  for (let index = 0; index < tokens.length; index += 1) {
    if (
      tokens[index].value !== "let" ||
      !identifierStart(tokens[index + 1]?.value?.[0])
    )
      continue;
    const local = tokens[index + 1].value;
    if (tokens[index + 2]?.value !== "=") continue;
    const path = [];
    let cursor = index + 3;
    while (identifierStart(tokens[cursor]?.value?.[0])) {
      path.push(tokens[cursor].value);
      cursor += 1;
      if (tokens[cursor]?.value !== "::") break;
      cursor += 1;
    }
    if (path.length > 0 && tokens[cursor]?.value === ";")
      aliases.set(local, path);
  }
  return aliases;
}

export function scanRust(source) {
  const tokenized = tokenizeRust(source);
  const errors = [...tokenized.errors];
  const aliases = aliasesFrom(tokenized.tokens, errors);
  const violations = [...errors];
  const forbidden = new Map([
    ["tokio::time::sleep", "rust-sleep"],
    ["tokio::time::sleep_until", "rust-sleep"],
    ["std::thread::sleep", "rust-sleep"],
    ["std::thread::park_timeout", "rust-sleep"],
    ["std::thread::yield_now", "rust-spin-wait"],
    ["tokio::task::yield_now", "rust-spin-wait"],
    ["std::hint::spin_loop", "rust-spin-wait"],
    ["core::hint::spin_loop", "rust-spin-wait"],
  ]);
  for (let index = 0; index < tokenized.tokens.length; index += 1) {
    if (tokenized.tokens[index - 1]?.value === "::") continue;
    if (!identifierStart(tokenized.tokens[index]?.value?.[0])) continue;
    const segments = [tokenized.tokens[index].value];
    let cursor = index + 1;
    while (tokenized.tokens[cursor]?.value === "::") {
      if (!identifierStart(tokenized.tokens[cursor + 1]?.value?.[0])) break;
      segments.push(tokenized.tokens[cursor + 1].value);
      cursor += 2;
    }
    if (tokenized.tokens[cursor]?.value !== "(") continue;
    const replacement = aliases.get(segments[0]);
    const canonical = (
      replacement === undefined
        ? segments
        : [...replacement, ...segments.slice(1)]
    ).join("::");
    const kind = forbidden.get(canonical);
    if (kind !== undefined)
      violations.push({ kind, line: tokenized.tokens[index].line });
  }
  return {
    comments: tokenized.comments,
    violations: violations.filter(
      (violation, index) =>
        violations.findIndex(
          (candidate) =>
            candidate.kind === violation.kind &&
            candidate.line === violation.line,
        ) === index,
    ),
  };
}

export function rustCfgTestSource(source) {
  const tokenized = tokenizeRust(source);
  if (tokenized.errors.length > 0) return source;
  const ranges = [];
  const tokens = tokenized.tokens;
  for (let index = 0; index < tokens.length - 9; index += 1) {
    const values = tokens.slice(index, index + 9).map(({ value }) => value);
    if (values.join(" ") !== "# [ cfg ( test ) ] mod") continue;
    let cursor = index + 9;
    if (!identifierStart(tokens[cursor]?.value?.[0])) continue;
    cursor += 1;
    if (tokens[cursor]?.value !== "{") continue;
    let depth = 1;
    const start = tokens[cursor].start;
    cursor += 1;
    while (cursor < tokens.length && depth > 0) {
      if (tokens[cursor].value === "{") depth += 1;
      if (tokens[cursor].value === "}") depth -= 1;
      cursor += 1;
    }
    if (depth === 0) ranges.push([start, tokens[cursor - 1].end]);
  }
  return source
    .split("")
    .map((character, index) =>
      character === "\n" ||
      ranges.some(([start, end]) => index >= start && index < end)
        ? character
        : " ",
    )
    .join("");
}
