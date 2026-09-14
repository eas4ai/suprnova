#!/usr/bin/env node
// Mechanism for UI-001, UI-002, UI-003, UI-004, UI-005, UI-007: the
// library's token stylesheet, base layer, headless styling, and the
// Tailwind preset.
//
//   node .cairn/tools/ui-tokens.mjs <stylesheet.css> [<views-root>] [<preset.css>] [<components-root>]
//
// The components root holds one directory per shipped component; every
// stylesheet under it is held to UI-003 and UI-004 like the base layer.
//
// Prints one `cairn: UI-nnn: pass|fail` line per requirement. A missing
// input fails its requirements with the reason; that is the honest state
// until the first commitment ships the files.

import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

const [stylesheet, viewsRoot, preset, componentsRoot] = process.argv.slice(2);
const results = new Map();
const fail = (id, why) => {
  const prior = results.get(id);
  results.set(id, prior && prior.startsWith("fail") ? `${prior}; ${why}` : `fail (${why}`);
};
const pass = (id) => results.has(id) || results.set(id, "pass");
const walk = (dir, out = []) => {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out); else out.push(p);
  }
  return out;
};

if (!stylesheet || !existsSync(stylesheet)) {
  const why = stylesheet ? `no stylesheet at ${stylesheet}` : "no stylesheet path given";
  for (const id of ["UI-001", "UI-002", "UI-003", "UI-004"]) fail(id, why);
} else {
  const css = readFileSync(stylesheet, "utf8");
  const decl = (name) => new RegExp(`--sn-${name}[a-z0-9-]*\\s*:`, "i");

  // UI-001: one --sn- custom property per semantic role, and a dark value set.
  const roles = ["color", "font", "space", "radius", "shadow", "motion", "density", "state"];
  for (const role of roles) if (!decl(role).test(css)) fail("UI-001", `no --sn- custom property for role ${role}`);
  if (!/prefers-color-scheme:\s*dark|\[data-theme=["']?dark/.test(css)) fail("UI-001", "no dark value set");
  pass("UI-001");

  // UI-002: a base rule for each bare semantic element, all inside the suprnova-ui layer.
  const elements = ["button", "input", "select", "textarea", "fieldset", "table", "details", "dialog", "a", "h1", "ul"];
  for (const el of elements) {
    if (!new RegExp(`(^|[\\s,}])${el}(\\s*[,{:\\[]|\\s+[a-z])`, "m").test(css)) fail("UI-002", `no base rule for ${el}`);
  }
  if (!/@layer\s+suprnova-ui\b/.test(css)) fail("UI-002", "no @layer suprnova-ui block");
  // Outside the layer only token definitions may remain: :root blocks, alone
  // or inside a media query. Anything else that still has a rule body is a
  // stray rule.
  const outsideLayer = css
    .replace(/@layer\s+suprnova-ui\s*\{[\s\S]*?\n\}/g, "")
    .replace(/:root\s*\{[^}]*\}/g, "")
    .replace(/@media[^{]*\{\s*\}/g, "");
  if (/[a-z\[.#][^{}]*\{[^}]*\}/i.test(outsideLayer)) fail("UI-002", "a rule sits outside @layer suprnova-ui");
  pass("UI-002");

  judgeStateAndLiterals(css, true);
}

// UI-003 and UI-004 over one stylesheet. The base layer must also carry the
// attribute selectors; a component stylesheet only must not select state by
// class or carry a literal visual value.
function judgeStateAndLiterals(css, isBase) {
  // UI-003: state presentation keyed to attributes, never to a state class alone.
  const stateClasses = /\.(is-|has-)?(invalid|busy|loading|expanded|open|pressed|active|current|selected|disabled)\b/g;
  const hits = [...css.matchAll(stateClasses)].map((m) => m[0]);
  if (hits.length) fail("UI-003", `state selected by class: ${[...new Set(hits)].join(" ")}`);
  const attrs = ["aria-invalid", "aria-busy", "aria-expanded", "aria-pressed", "aria-current", "aria-selected", ":disabled"];
  if (isBase) for (const attr of attrs) if (!css.includes(attr)) fail("UI-003", `no selector on ${attr}`);
  pass("UI-003");

  // UI-004: no literal visual value outside the token definitions themselves.
  // Parse declarations and judge each value, so a `var(` guard cannot be
  // backtracked past.
  const body = css.replace(/:root\s*\{[^}]*\}/g, "").replace(/--sn-[a-z0-9-]+\s*:[^;]+;/g, "");
  const visual = new Set(["color", "background", "background-color", "border-color", "outline-color", "fill", "stroke", "font-family", "font", "border-radius", "box-shadow", "text-shadow", "transition", "transition-duration", "transition-timing-function", "animation", "animation-duration", "animation-timing-function"]);
  const seen = new Set();
  for (const m of body.matchAll(/([a-z-]+)\s*:\s*([^;{}]+)/gi)) {
    const prop = m[1].toLowerCase(), value = m[2].trim();
    if (!visual.has(prop)) continue;
    const tokenOnly = /^(var\(--sn-[a-z0-9-]+\)|inherit|initial|unset|none|transparent|currentcolor|0)$/i.test(value)
      || value.split(/\s+/).every((part) => /^(var\(--sn-[a-z0-9-]+\)|inherit|initial|unset|none|transparent|currentcolor|0|[a-z-]+)$/i.test(part) && !/^#|^(rgb|hsl|oklch|oklab|lab|lch)a?\(/i.test(part));
    if (!tokenOnly || /#[0-9a-f]{3,8}\b|\b(rgb|hsl|oklch|oklab|lab|lch)a?\(|\d+(\.\d+)?(px|rem|em|ms|s)\b|\bsystem-ui\b|\bserif\b|\bsans-serif\b|\bmonospace\b/i.test(value)) seen.add(`${prop} literal`);
  }
  for (const why of seen) fail("UI-004", why);
  pass("UI-004");
}

if (componentsRoot && existsSync(componentsRoot)) {
  for (const file of walk(componentsRoot)) {
    if (file.endsWith(".css")) judgeStateAndLiterals(readFileSync(file, "utf8"), false);
  }
}

// UI-005: no style attribute in any shipped view.
if (!viewsRoot || !existsSync(viewsRoot)) {
  fail("UI-005", viewsRoot ? `no views root at ${viewsRoot}` : "no views root given");
} else {
  for (const file of walk(viewsRoot)) {
    if (!file.endsWith(".html")) continue;
    if (/\sstyle\s*=/.test(readFileSync(file, "utf8"))) fail("UI-005", `style attribute in ${file}`);
  }
  pass("UI-005");
}

// UI-007: the Tailwind preset maps every color, space, radius, and font token.
if (!preset || !existsSync(preset)) {
  fail("UI-007", preset ? `no preset at ${preset}` : "no preset path given");
} else if (stylesheet && existsSync(stylesheet)) {
  const css = readFileSync(stylesheet, "utf8"), pre = readFileSync(preset, "utf8");
  const tokens = [...css.matchAll(/--sn-(color|space|radius|font)-[a-z0-9-]+/g)].map((m) => m[0]);
  for (const t of new Set(tokens)) if (!pre.includes(t)) fail("UI-007", `preset does not map ${t}`);
  if (!/@theme\b/.test(pre)) fail("UI-007", "preset has no @theme block");
  pass("UI-007");
} else {
  fail("UI-007", "no stylesheet to compare the preset against");
}

// Cairn reads the bare `cairn: <id>: pass|fail` line; the reasons follow on
// their own indented line so a reader has them without breaking the parse.
let failed = false;
for (const [id, result] of results) {
  if (result.startsWith("fail")) {
    failed = true;
    process.stdout.write(`cairn: ${id}: fail\n  reason: ${result.slice("fail (".length)}\n`);
  } else {
    process.stdout.write(`cairn: ${id}: pass\n`);
  }
}
process.exit(failed ? 1 : 0);
