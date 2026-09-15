#!/usr/bin/env node
// Mechanism for DATA-001, DATA-002 and DATA-004: the data-display
// components and the dogfood gallery that mounts them.
//
//   node .cairn/tools/ui-data-display.mjs <components-root> <gallery-view> <browser-src>
//
// DATA-001: no layout primitive (separator, scroll-area, aspect-image,
// card, description-list) wraps its content in an unlabeled generic
// element or reorders it with CSS; every region and group carries a label.
// DATA-002: badge, avatar and stat views render text or an aria-label for
// every variant, and the stat's trend is named in text.
// DATA-004: the chart view carries a summary and a data table beside its
// marks, ships no script, and the browser source holds no charting
// library; the gallery mounts a chart and renders its SVG on the server.
//
// Prints one `cairn: DATA-nnn: pass|fail` line per requirement.

import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

const [root, gallery, browserSrc] = process.argv.slice(2);
const IDS = ["DATA-001", "DATA-002", "DATA-004"];
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
const read = (name, ext) => {
  const p = join(root, name, `${name}.${ext}`);
  return existsSync(p) ? readFileSync(p, "utf8") : null;
};

if (!root || !existsSync(root)) {
  for (const id of IDS) fail(id, root ? `no components root at ${root}` : "no components root given");
} else {
  // DATA-001: layout primitives.
  for (const name of ["separator", "scroll-area", "aspect-image", "card", "description-list"]) {
    const html = read(name, "html");
    const css = read(name, "css") ?? "";
    if (html === null) { fail("DATA-001", `no ${name} view`); continue; }
    if (/\border\s*:/.test(css) || /flex-direction\s*:\s*(row|column)-reverse/.test(css)) fail("DATA-001", `${name} reorders content with CSS`);
    for (const m of html.matchAll(/<(div|span)\b([^>]*)>/g)) {
      const attrs = m[2];
      const labeled = /\brole="(region|group|separator|img)"/.test(attrs) && /\baria-label(ledby)?=/.test(attrs);
      const structural = name === "description-list" && /sn-description\b/.test(attrs);
      const marks = name === "chart";
      if (!labeled && !structural && !marks && !/aria-hidden="true"/.test(attrs) && !/sn-card-actions/.test(attrs)) fail("DATA-001", `${name}: an unlabeled generic ${m[1]} wraps content`);
      if (/sn-card-actions/.test(attrs) && !/role="group"/.test(attrs)) fail("DATA-001", "card actions are not a labeled group");
    }
    if (name === "scroll-area" && !/tabindex="0"/.test(html)) fail("DATA-001", "the scroll area is not keyboard focusable");
    if (name === "aspect-image" && /<div|<span/.test(html)) fail("DATA-001", "the aspect image adds a wrapper");
    if (name === "card" && !/aria-labelledby="\{\{ id \}\}-title"/.test(html)) fail("DATA-001", "the card is not labeled by its heading");
  }
  pass("DATA-001");

  // DATA-002: badge, avatar, stat.
  const badge = read("badge", "html");
  if (badge === null) fail("DATA-002", "no badge view");
  else if (!/<span class="sn-badge"[^>]*>\{\{ text \}\}<\/span>/.test(badge)) fail("DATA-002", "the badge does not render its text");
  const avatar = read("avatar", "html");
  if (avatar === null) fail("DATA-002", "no avatar view");
  else {
    if (!/<img class="sn-avatar"[^>]*alt="\{\{ name \}\}"/.test(avatar)) fail("DATA-002", "the image avatar has no alt naming the person");
    if (!/role="img" aria-label="\{\{ name \}\}"[^>]*>\{\{ initials \}\}/.test(avatar)) fail("DATA-002", "the initials fallback is not labeled");
  }
  const stat = read("stat-card", "html");
  if (stat === null) fail("DATA-002", "no stat-card view");
  else {
    if (!/<data value="\{\{ value \}\}">/.test(stat)) fail("DATA-002", "the stat value is not a data element");
    if (!/sn-stat-direction/.test(stat) || !/Up\{% elif trend == "down" %\}Down\{% else %\}Flat/.test(stat)) fail("DATA-002", "the trend direction is not named in text");
  }
  pass("DATA-002");

  // DATA-004: chart.
  const chart = read("chart", "html");
  if (chart === null) fail("DATA-004", "no chart view");
  else {
    if (!/svg\|trusted_html/.test(chart)) fail("DATA-004", "the chart does not render server-rendered marks");
    if (!/sn-chart-summary/.test(chart)) fail("DATA-004", "the chart has no text summary");
    if (!/<details class="sn-chart-data">/.test(chart) || !/\{\{ caller\(\) \}\}/.test(chart)) fail("DATA-004", "the chart has no data table alternative");
    if (existsSync(join(root, "chart", "chart.js"))) fail("DATA-004", "the chart ships a script");
  }
  const charting = /\b(chart\.js|chartjs|echarts|d3|plotly|highcharts|apexcharts|recharts|vega|uplot)\b/i;
  const views = walk(root).filter((p) => /\.(html|js|css)$/.test(p));
  for (const p of views) if (charting.test(readFileSync(p, "utf8"))) fail("DATA-004", `a charting library is named in ${p.slice(root.length + 1)}`);
  if (!browserSrc || !existsSync(browserSrc)) fail("DATA-004", browserSrc ? `no browser source at ${browserSrc}` : "no browser source given");
  else for (const p of walk(browserSrc).filter((p) => /\.(ts|js|mjs)$/.test(p) && !/generated/.test(p))) {
    if (charting.test(readFileSync(p, "utf8"))) fail("DATA-004", `a charting library appears in ${p.slice(browserSrc.length + 1)}`);
  }
  pass("DATA-004");
}

if (!gallery || !existsSync(gallery)) {
  for (const id of IDS) fail(id, gallery ? `no gallery view at ${gallery}` : "no gallery view given");
} else {
  const view = readFileSync(gallery, "utf8");
  const alias = (name) => {
    const m = view.match(new RegExp(`\\{%\\s*import\\s+"suprnova-ui/${name}/${name}\\.html"\\s+as\\s+(\\w+)\\s*%\\}`));
    return m ? m[1] : name.replace(/-/g, "_");
  };
  const mounts = (name, macro) => new RegExp(`${alias(name)}::${macro}\\(`).test(view);
  for (const [name, macro] of [["separator", "separator"], ["scroll-area", "scroll_area"], ["aspect-image", "aspect_image"], ["card", "card"], ["description-list", "description_list"]]) {
    if (!mounts(name, macro)) fail("DATA-001", `the gallery mounts no ${name}`);
  }
  for (const [name, macro] of [["badge", "badge"], ["avatar", "avatar"], ["avatar", "avatar_group"], ["stat-card", "stat_card"]]) {
    if (!mounts(name, macro)) fail("DATA-002", `the gallery mounts no ${macro}`);
  }
  if (!/trend="up"/.test(view) || !/trend="down"/.test(view)) fail("DATA-002", "the gallery shows only one trend direction");
  if (!mounts("chart", "chart")) fail("DATA-004", "the gallery mounts no chart");
  if (!/<script/.test(view) === false) fail("DATA-004", "the gallery carries a script");
}

for (const id of IDS) {
  const r = results.get(id) ?? "fail (not judged";
  if (r === "pass") console.log(`cairn: ${id}: pass`);
  else console.log(`cairn: ${id}: fail\n  reason: ${r.slice(6)}`);
}
process.exit([...results.values()].every((r) => r === "pass") ? 0 : 1);
