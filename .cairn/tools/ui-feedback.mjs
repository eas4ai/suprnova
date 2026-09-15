#!/usr/bin/env node
// Mechanism for FDB-001, FDB-002 and FDB-006 over the shipped feedback
// components and the dogfood gallery that mounts them.
//
//   node .cairn/tools/ui-feedback.mjs <components-root> <gallery-view>
//
// FDB-001: the alert chooses its role from its variant, and every variant
// carries a non-color cue: an icon glyph and a hidden text label that
// differ between variants, so no variant differs from its siblings only in
// color tokens.
// FDB-002: every spinner and skeleton root is bound through a live:loading
// state directive or declared as a lazy island placeholder, no feedback
// script runs a timer of its own, and every gallery mount passes an action
// or asks for the placeholder.
// FDB-006: the progress view is a native progress element with max, a value
// only for determinate work, a label, and a text readout of completion;
// no bar is drawn from a width value.
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const [root, gallery] = process.argv.slice(2);
const IDS = ["FDB-001", "FDB-002", "FDB-006"];
const results = new Map(IDS.map((id) => [id, "pass"]));
const fail = (id, why) => {
  const prior = results.get(id);
  results.set(id, prior.startsWith("fail") ? `${prior}; ${why}` : `fail (${why}`);
};
const read = (dir, name) => {
  const p = join(root, dir, name);
  return existsSync(p) ? readFileSync(p, "utf8") : null;
};
const VARIANTS = ["info", "success", "warning", "error"];

if (!root || !existsSync(root)) {
  for (const id of IDS) fail(id, root ? `no components root at ${root}` : "no components root given");
} else {
  const present = readdirSync(root);
  // FDB-001
  if (!present.includes("alert")) fail("FDB-001", "no alert component");
  else {
    const html = read("alert", "alert.html") ?? "";
    if (!/\{%\s*if variant == "error"\s*%\}role="alert"/.test(html) || !/role="status"/.test(html)) {
      fail("FDB-001", "the alert role is not chosen by variant");
    }
    for (const variant of VARIANTS) {
      const branches = [...html.matchAll(new RegExp(`\\{%\\s*(?:if|elif) variant == "${variant}"\\s*%\\}([\\s\\S]*?)\\{%\\s*(?:elif|else|endif)`, "g"))];
      if (branches.length === 0) { fail("FDB-001", `no ${variant} branch in the alert view`); continue; }
      if (!branches.some((b) => /aria-hidden="true"/.test(b[1]) && /sn-alert-label/.test(b[1]))) {
        fail("FDB-001", `the ${variant} variant carries no icon and hidden label`);
      }
    }
    const labels = [...html.matchAll(/<span class="sn-alert-label">([^<]+)<\/span>/g)].map((m) => m[1]);
    if (new Set(labels).size < VARIANTS.length) fail("FDB-001", "variant labels are not distinct");
    if (read("alert", "alert.js") !== null) fail("FDB-001", "the alert ships a script");
  }
  // FDB-002
  for (const name of ["spinner", "skeleton"]) {
    if (!present.includes(name)) { fail("FDB-002", `no ${name} component`); continue; }
    const html = read(name, `${name}.html`) ?? "";
    if (!/live:loading\.show="\{\{ action \}\}"/.test(html)) fail("FDB-002", `${name} is not bound through live:loading.show`);
    if (!/data-sn-placeholder="lazy"/.test(html)) fail("FDB-002", `${name} has no lazy island placeholder form`);
    if (!/aria-busy="true"|role="status"/.test(html)) fail("FDB-002", `${name} carries no busy or status semantics`);
    const js = read(name, `${name}.js`);
    if (js !== null) fail("FDB-002", `${name} ships a script`);
    const css = read(name, `${name}.css`) ?? "";
    if (/animation-delay|transition-delay/.test(css)) fail("FDB-002", `${name}.css runs its own anti-flicker delay`);
  }
  // FDB-006
  if (!present.includes("progress")) fail("FDB-006", "no progress component");
  else {
    const html = read("progress", "progress.html") ?? "";
    if (!/<progress\b[^>]*\bmax="\{\{ max \}\}"/.test(html)) fail("FDB-006", "the progress view is not a native progress element with max");
    if (!/\{%\s*if determinate\s*%\}\s*value="\{\{ value \}\}"/.test(html)) fail("FDB-006", "value is not conditional on determinate work");
    if (!/<label\b[^>]*\bfor="\{\{ id \}\}"/.test(html)) fail("FDB-006", "the progress has no label");
    if (!/sn-progress-readout/.test(html)) fail("FDB-006", "the progress has no text readout");
    const css = read("progress", "progress.css") ?? "";
    if (/width:\s*calc|width:\s*var\(--sn-progress/.test(css)) fail("FDB-006", "the bar is drawn from a width value");
    if (/\sstyle=/.test(html)) fail("FDB-006", "the progress view carries a style attribute");
  }
}
if (!gallery || !existsSync(gallery)) {
  for (const id of IDS) fail(id, gallery ? `no gallery view at ${gallery}` : "no gallery view given");
} else {
  const view = readFileSync(gallery, "utf8");
  const mounted = (needle) => new RegExp(`\\b${needle}\\(`).test(view);
  for (const variant of VARIANTS) {
    if (!view.includes(`alert::alert(`) || !view.includes(`"${variant}"`)) fail("FDB-001", `the gallery mounts no ${variant} alert`);
  }
  for (const name of ["spinner", "skeleton"]) {
    const calls = [...view.matchAll(new RegExp(`${name}::${name}\\(([^)]*)\\)`, "g"))];
    if (calls.length === 0) fail("FDB-002", `the gallery mounts no ${name}`);
    for (const call of calls) {
      if (!/action="[a-z_]+"/.test(call[1]) && !/lazy=true/.test(call[1])) fail("FDB-002", `${name} mounted with no action and no lazy placeholder: ${call[0]}`);
    }
  }
  if (!mounted("progress::progress")) fail("FDB-006", "the gallery mounts no progress");
  else if (!/determinate=true/.test(view) || !/determinate=false/.test(view)) fail("FDB-006", "the gallery mounts only one progress kind");
}
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
