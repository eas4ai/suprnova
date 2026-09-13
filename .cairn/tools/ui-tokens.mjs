#!/usr/bin/env node
// Mechanism for UI-001, UI-002, UI-003: the library's token stylesheet.
//
//   node .cairn/tools/ui-tokens.mjs <stylesheet.css>
//
// Prints one `cairn: UI-nnn: pass|fail` line per requirement. A missing
// stylesheet fails all three with the reason; that is the honest state
// until the first commitment ships the file.

import { existsSync, readFileSync } from "node:fs";

const path = process.argv[2];
const results = new Map();
const fail = (id, why) => {
  const prior = results.get(id);
  results.set(id, prior ? `${prior}; ${why}` : `fail (${why}`);
};
const pass = (id) => results.has(id) || results.set(id, "pass");

if (!path || !existsSync(path)) {
  const why = path ? `no stylesheet at ${path}` : "no stylesheet path given";
  for (const id of ["UI-001", "UI-002", "UI-003"]) fail(id, why);
} else {
  const css = readFileSync(path, "utf8");
  const decl = (name) => new RegExp(`--${name}[a-z0-9-]*\\s*:`, "i");

  // UI-001: one custom property per semantic role, and a dark value set.
  const roles = ["color", "font", "space", "radius", "shadow", "motion", "density", "state"];
  for (const role of roles) if (!decl(role).test(css)) fail("UI-001", `no custom property for role ${role}`);
  if (!/prefers-color-scheme:\s*dark|\[data-theme=["']?dark/.test(css)) fail("UI-001", "no dark value set");
  pass("UI-001");

  // UI-002: a base-layer rule for each bare semantic element.
  const elements = ["button", "input", "select", "textarea", "fieldset", "table", "details", "dialog", "a", "h1", "ul"];
  for (const el of elements) {
    if (!new RegExp(`(^|[\\s,}])${el}(\\s*[,{:\\[]|\\s+[a-z])`, "m").test(css)) fail("UI-002", `no base rule for ${el}`);
  }
  pass("UI-002");

  // UI-003: state presentation keyed to attributes, never to a state class alone.
  const stateClasses = /\.(is-|has-)?(invalid|busy|loading|expanded|open|pressed|active|current|selected|disabled)\b/g;
  const hits = [...css.matchAll(stateClasses)].map((m) => m[0]);
  if (hits.length) fail("UI-003", `state selected by class: ${[...new Set(hits)].join(" ")}`);
  const attrs = ["aria-invalid", "aria-busy", "aria-expanded", "aria-pressed", "aria-current", "aria-selected", ":disabled"];
  for (const attr of attrs) if (!css.includes(attr)) fail("UI-003", `no selector on ${attr}`);
  pass("UI-003");
}

let failed = false;
for (const [id, result] of results) {
  const line = result.startsWith("fail") ? `${result})` : result;
  process.stdout.write(`cairn: ${id}: ${line}\n`);
  if (result.startsWith("fail")) failed = true;
}
process.exit(failed ? 1 : 0);
