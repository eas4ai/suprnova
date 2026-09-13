// Shared specification grammar. A requirement is one contiguous paragraph;
// its Status overrides the file header. Fenced examples are not requirements.
export function withoutFences(text) {
  let fence = null;
  return text.split(/\r?\n/).map((line) => {
    if (fence) {
      if (new RegExp(`^ {0,3}${fence[0]}{${fence.length},}\\s*$`).test(line)) fence = null;
      return "";
    }
    const m = /^ {0,3}(`{3,}|~{3,})/.exec(line);
    if (m) { fence = m[1]; return ""; }
    return line;
  });
}

export function parseSpec(text) {
  const lines = withoutFences(text), blocks = [];
  const first = lines.findIndex((line) => /^\[[A-Z]+-\d+\]/.test(line));
  const header = lines.slice(0, first < 0 ? lines.length : first).join("\n");
  const status = /^Status:[ \t]*(\w+)/m.exec(header)?.[1] ?? null;
  const prefix = /^Prefix:[ \t]*([A-Z]+)/m.exec(header)?.[1] ?? null;
  const scope = /^Scope:[ \t]*(.*)$/m.exec(header)?.[1].trim() ?? null;
  for (let i = 0; i < lines.length; i++) {
    const m = /^\[([A-Z]+-\d+)\]\s*(.*)$/.exec(lines[i]);
    if (!m) continue;
    const start = i, body = [m[2]];
    while (i + 1 < lines.length && lines[i + 1].trim() && !/^(?:\[[A-Z]+-\d+\]|#)/.test(lines[i + 1])) body.push(lines[++i]);
    const ownStatus = body.find((line) => /^Status:/.test(line));
    blocks.push({ id: m[1], line: start + 1, body,
      status: ownStatus === undefined ? status : /^Status:[ \t]*(\w+)/.exec(ownStatus)?.[1] ?? null,
      falsifier: body.some((line) => /^Falsifier:/.test(line)) });
  }
  return { lines, blocks, status, prefix, scope };
}
