#!/usr/bin/env node
// Mechanism for NAV-001, NAV-002 and NAV-004 over the shipped navigation
// components and the dogfood gallery that mounts them.
//
//   node .cairn/tools/ui-navigation.mjs <components-root> <gallery-view>
//
// NAV-001: in every navigation view an anchor has an href and no Live
// directive, and no button carries an href; the load-more control is a
// button on a registered action.
// NAV-002: the tabs view has a local branch with tablist, tab and tabpanel
// roles and no Live directive, a route branch of anchors with aria-current,
// and every gallery mount declares one of the two modes.
// NAV-004: the header bar and sidebar take aria-current from a server-bound
// value, ship no script that reads the location or writes aria-current, and
// the sidebar's collapsible groups are native details.
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const [root, gallery] = process.argv.slice(2);
const IDS = ["NAV-001", "NAV-002", "NAV-004"];
const NAVIGATION = ["header-bar", "footer", "sidebar", "breadcrumbs", "tabs", "pagination", "load-more"];
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
  // NAV-001
  for (const name of NAVIGATION) {
    if (!present.includes(name)) { fail("NAV-001", `no ${name} component`); continue; }
    const html = read(name, `${name}.html`) ?? "";
    for (const { tag, attrs } of tags(html)) {
      if (tag === "a" && !/\bhref=/.test(attrs)) fail("NAV-001", `${name}: an anchor without href`);
      if (tag === "a" && /\slive:/.test(attrs)) fail("NAV-001", `${name}: an anchor carries a Live directive`);
      if (tag === "button" && /\bhref=/.test(attrs)) fail("NAV-001", `${name}: a button carries an href`);
    }
  }
  const more = read("load-more", "load-more.html") ?? "";
  if (more && !/<button\b[^>]*\blive:click="\{\{ action \}\}"/.test(more)) fail("NAV-001", "the load-more control is not a button on a registered action");
  // NAV-002
  const tabs = read("tabs", "tabs.html");
  if (tabs === null) fail("NAV-002", "no tabs view");
  else {
    const local = tabs.match(/\{%\s*if mode == "local"\s*%\}([\s\S]*?)\{%\s*(?:elif|else)/);
    const route = tabs.match(/\{%\s*(?:elif|if) mode == "route"\s*%\}([\s\S]*?)\{%\s*(?:elif|else|endif)/);
    const routeTab = tabs.match(/\{%\s*macro route_tab[^%]*%\}([\s\S]*?)\{%\s*endmacro\s*%\}/);
    if (!local) fail("NAV-002", "the tabs view has no local branch");
    else {
      for (const role of ["tablist", "tab", "tabpanel"]) if (!tabs.includes(`role="${role}"`)) fail("NAV-002", `no ${role} role in the local mode`);
      if (/\slive:/.test(local[1])) fail("NAV-002", "a local tab carries a Live directive");
    }
    if (!route) fail("NAV-002", "the tabs view has no route branch");
    else if (/\slive:/.test(route[1])) fail("NAV-002", "the route branch carries a Live directive");
    if (!routeTab || !/<a\b[^>]*\bhref=/.test(routeTab[1]) || !/aria-current/.test(routeTab[1])) fail("NAV-002", "route tabs are not anchors with aria-current");
  }
  // NAV-004
  for (const name of ["header-bar", "sidebar"]) {
    if (!present.includes(name)) { fail("NAV-004", `no ${name} component`); continue; }
    const html = read(name, `${name}.html`) ?? "";
    if (!/\{%\s*if current\s*%\}\s*aria-current="page"/.test(html)) fail("NAV-004", `${name} does not take aria-current from a bound value`);
    const js = read(name, `${name}.js`);
    if (js !== null && /location\.|aria-current|URL\(/.test(js)) fail("NAV-004", `${name}.js computes the current item in the browser`);
  }
  const sidebar = read("sidebar", "sidebar.html") ?? "";
  if (sidebar && !/<details\b/.test(sidebar)) fail("NAV-004", "sidebar groups do not collapse through details");
}
if (!gallery || !existsSync(gallery)) {
  for (const id of IDS) fail(id, gallery ? `no gallery view at ${gallery}` : "no gallery view given");
} else {
  const view = readFileSync(gallery, "utf8");
  for (const name of NAVIGATION) {
    const macro = name.replace(/-/g, "_");
    const imported = view.match(new RegExp(`\\{%\\s*import\\s+"suprnova-ui/${name}/${name}\\.html"\\s+as\\s+(\\w+)\\s*%\\}`));
    const alias = imported ? imported[1] : macro;
    if (!new RegExp(`${alias}::${macro}\\(`).test(view)) fail("NAV-001", `the gallery mounts no ${name}`);
  }
  const calls = [...view.matchAll(/tabs::tabs\(([^)]*)\)/g)];
  if (calls.length === 0) fail("NAV-002", "the gallery mounts no tabs");
  for (const call of calls) if (!/mode="(local|route)"/.test(call[1])) fail("NAV-002", `tabs mounted without a mode: ${call[0]}`);
  if (!calls.some((c) => c[1].includes('mode="local"')) || !calls.some((c) => c[1].includes('mode="route"'))) fail("NAV-002", "the gallery mounts only one tabs mode");
  if (!/current=true/.test(view)) fail("NAV-004", "the gallery binds no current item");
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
