#!/usr/bin/env node
// Mechanism for UI-013 over the shipped and vendored library views: a
// behavioral component mounts one island per widget, never one per row or
// cell, so no `live:component` mount may sit inside a `{% for %}` loop in a
// library view.
//
//   node .cairn/tools/ui-islands.mjs <root> [<root> ...]
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

const roots = process.argv.slice(2);
let reason = null;
const walk = (dir, out = []) => {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out); else out.push(p);
  }
  return out;
};
if (roots.length === 0) reason = "no roots given";
for (const root of roots) {
  if (!existsSync(root)) { reason = `no root at ${root}`; continue; }
  for (const file of walk(root)) {
    if (!file.endsWith(".html")) continue;
    const source = readFileSync(file, "utf8");
    let depth = 0;
    for (const token of source.matchAll(/\{%-?\s*(for|endfor)\b|live:component\b/g)) {
      if (token[1] === "for") depth += 1;
      else if (token[1] === "endfor") depth = Math.max(0, depth - 1);
      else if (depth > 0) reason = `${file} mounts an island per loop item`;
    }
  }
}
if (reason) {
  process.stdout.write(`cairn: UI-013: fail\n  reason: ${reason}\n`);
  process.exit(1);
}
process.stdout.write("cairn: UI-013: pass\n");
