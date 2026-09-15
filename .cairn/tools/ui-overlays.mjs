#!/usr/bin/env node
// Mechanism for OVL-001 to OVL-004 over the shipped overlay components and
// the dogfood gallery that mounts them.
//
//   node .cairn/tools/ui-overlays.mjs <components-root> <gallery-view>
//
// OVL-001: every disclosure, popover and dialog root is the native primitive
// (details, the popover attribute, dialog) and no vendored script owns open
// state or focus containment: no script writes the open or hidden attribute,
// toggles a class for open state, or handles Tab to trap focus.
// OVL-002: the tooltip ships no script, shows on :hover and :focus-visible,
// and the gallery associates every tooltip through aria-describedby.
// OVL-003: the menu is single level (no list inside an item), navigation
// items are anchors and action items are buttons on a registered action.
// OVL-004: no open or close control carries a live: directive, so opening and
// closing an overlay makes no Live request; only declared actions do.
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const [root, gallery] = process.argv.slice(2);
const IDS = ["OVL-001", "OVL-002", "OVL-003", "OVL-004"];
const results = new Map(IDS.map((id) => [id, "pass"]));
const fail = (id, why) => {
  const prior = results.get(id);
  results.set(id, prior.startsWith("fail") ? `${prior}; ${why}` : `fail (${why}`);
};
const read = (dir, name) => {
  const p = join(root, dir, name);
  return existsSync(p) ? readFileSync(p, "utf8") : null;
};
const tags = (html) => [...html.matchAll(/<([a-z][a-z0-9-]*)\b([^>]*)>/g)].map((m) => ({ tag: m[1], attrs: m[2] }));

if (!root || !existsSync(root)) {
  for (const id of IDS) fail(id, root ? `no components root at ${root}` : "no components root given");
} else {
  const present = readdirSync(root);
  const native = {
    collapsible: /<details\b/,
    accordion: /<details\b[^>]*\bname=/,
    popover: /<div\b[^>]*\bpopover\b/,
    "dropdown-menu": /<div\b[^>]*\bpopover\b/,
    dialog: /<dialog\b/,
    sheet: /<dialog\b/,
    drawer: /<dialog\b/,
  };
  for (const [name, pattern] of Object.entries(native)) {
    if (!present.includes(name)) { fail("OVL-001", `no ${name} component`); continue; }
    const html = read(name, `${name}.html`) ?? "";
    if (!pattern.test(html)) fail("OVL-001", `${name} does not open through its native primitive`);
    const js = read(name, `${name}.js`);
    if (js !== null) {
      if (/setAttribute\(\s*["'](open|hidden)["']|\.open\s*=|\.hidden\s*=|classList\.(add|remove|toggle)\(/.test(js)) {
        fail("OVL-001", `${name}.js owns open state in script`);
      }
      if (/["']Tab["']|activeElement/.test(js) && /keydown/.test(js)) fail("OVL-001", `${name}.js traps focus in script`);
      if (/inert/.test(js)) fail("OVL-001", `${name}.js manages inertness in script`);
    }
    for (const { tag, attrs } of tags(html)) {
      const control = /\bpopovertarget=|data-sn-[a-z]+-(open|close)=/.test(attrs) || tag === "summary";
      if (control && /\slive:(?!click="\{\{ action \}\}")/.test(attrs)) fail("OVL-004", `${name}: an open or close control carries a Live directive: <${tag}${attrs}>`);
    }
    if (js !== null && /fetch\(|XMLHttpRequest|__live/.test(js)) fail("OVL-004", `${name}.js talks to the server`);
  }
  // OVL-002
  if (!present.includes("tooltip")) fail("OVL-002", "no tooltip component");
  else {
    if (read("tooltip", "tooltip.js") !== null) fail("OVL-002", "the tooltip ships a script");
    const css = read("tooltip", "tooltip.css") ?? "";
    for (const needle of [":hover", ":focus-visible"]) if (!css.includes(needle)) fail("OVL-002", `tooltip.css has no ${needle} rule`);
    if (!/role="tooltip"/.test(read("tooltip", "tooltip.html") ?? "")) fail("OVL-002", "the bubble has no tooltip role");
  }
  // OVL-003
  const menu = read("dropdown-menu", "dropdown-menu.html") ?? "";
  if (!menu) fail("OVL-003", "no dropdown-menu view");
  else {
    if (/<li\b[^>]*>(?:(?!<\/li>).)*<ul\b/s.test(menu)) fail("OVL-003", "a menu item nests a list");
    const link = menu.match(/\{% macro menu_link[^%]*%\}([\s\S]*?)\{% endmacro %\}/);
    const action = menu.match(/\{% macro menu_action[^%]*%\}([\s\S]*?)\{% endmacro %\}/);
    if (!link || !/<a\b[^>]*\bhref=/.test(link[1])) fail("OVL-003", "navigation items are not anchors");
    if (!action || !/<button\b[^>]*\blive:click=/.test(action[1]) || /<a\b/.test(action[1])) fail("OVL-003", "action items are not buttons on a registered action");
  }
}
if (!gallery || !existsSync(gallery)) {
  fail("OVL-002", gallery ? `no gallery view at ${gallery}` : "no gallery view given");
  fail("OVL-003", gallery ? `no gallery view at ${gallery}` : "no gallery view given");
} else {
  const view = readFileSync(gallery, "utf8");
  for (const m of view.matchAll(/tooltip::tooltip\("([^"]+)"/g)) {
    if (!view.includes(`aria-describedby="${m[1]}"`)) fail("OVL-002", `tooltip ${m[1]} is not referenced by aria-describedby`);
  }
  if (!/tooltip::tooltip\(/.test(view)) fail("OVL-002", "the gallery mounts no tooltip");
  const menus = [...view.matchAll(/\{% call menu::menu\([\s\S]*?\{% endcall %\}/g)];
  if (menus.length === 0) fail("OVL-003", "the gallery mounts no menu");
  for (const m of menus) {
    const inner = m[0].slice(m[0].indexOf("%}") + 2, -"{% endcall %}".length);
    if (/menu::menu\(/.test(inner)) fail("OVL-003", "the gallery nests a menu in a menu");
  }
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
