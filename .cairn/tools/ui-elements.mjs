#!/usr/bin/env node
// Mechanism for UI-011 and UI-018 over the shipped components' JavaScript.
//
//   node .cairn/tools/ui-elements.mjs <components-root>
//
// UI-018: every custom element a component defines carries the `sn-` prefix,
// and only a component's own vendored file defines it (a definition outside
// a component directory is a violation). UI-010 is checked alongside: no
// shadow root. UI-011: an element that carries a form value (it calls
// setFormValue through ElementInternals) declares `static formAssociated`,
// so live:model and validation see a real control.
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

const [root] = process.argv.slice(2);
const results = new Map([["UI-011", "pass"], ["UI-018", "pass"]]);
const fail = (id, why) => {
  const prior = results.get(id);
  results.set(id, prior && prior.startsWith("fail") ? `${prior}; ${why}` : `fail (${why}`);
};
const walk = (dir, out = []) => {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out); else out.push(p);
  }
  return out;
};
if (!root || !existsSync(root)) {
  fail("UI-011", root ? `no components root at ${root}` : "no components root given");
  fail("UI-018", root ? `no components root at ${root}` : "no components root given");
} else {
  let elements = 0;
  for (const file of walk(root)) {
    if (!file.endsWith(".js")) continue;
    const js = readFileSync(file, "utf8");
    for (const m of js.matchAll(/customElements\.define\(\s*["']([^"']+)["']/g)) {
      elements += 1;
      if (!m[1].startsWith("sn-")) fail("UI-018", `${file} defines ${m[1]} without the sn- prefix`);
      const manifest = JSON.parse(readFileSync(join(file, "..", "manifest.json"), "utf8"));
      if (!Array.isArray(manifest.elements) || !manifest.elements.includes(m[1])) {
        fail("UI-018", `${file} defines ${m[1]} but its manifest does not declare it`);
      }
    }
    if (/attachShadow/.test(js)) fail("UI-018", `${file} attaches a shadow root`);
    const carriesValue = /setFormValue\s*\(/.test(js);
    const formAssociated = /static\s+formAssociated\s*=\s*true/.test(js);
    if (carriesValue && !formAssociated) fail("UI-011", `${file} sets a form value without static formAssociated`);
    if (formAssociated && !/attachInternals\s*\(/.test(js)) fail("UI-011", `${file} is form-associated without ElementInternals`);
  }
  process.stdout.write(`  elements defined: ${elements}\n`);
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
