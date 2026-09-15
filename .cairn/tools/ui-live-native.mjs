#!/usr/bin/env node
// Mechanism for FORM-006 and FORM-007 over the shipped live-native views and
// their element scripts.
//
//   node .cairn/tools/ui-live-native.mjs <components-root> <gallery-view>
//
// FORM-006: the input OTP is one native input (inputmode numeric,
// autocomplete one-time-code, a length pattern, a model binding) with
// aria-hidden presentation cells; its script defines sn-input-otp and nothing
// else, attaches no shadow root, carries no form value, and only reads the
// input. FORM-007: the date picker's input is type date and carries the model;
// its year, month and day strips are fieldsets of native radio inputs inside a
// scroll-snap container, so a strip selection needs no script; its script
// defines sn-date-picker alone. Both are exercised in the gallery view.
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

const [root, gallery] = process.argv.slice(2);
const IDS = ["FORM-006", "FORM-007"];
const results = new Map(IDS.map((id) => [id, "pass"]));
const fail = (id, why) => {
  const prior = results.get(id);
  results.set(id, prior === "pass" ? `fail (${why}` : `${prior}; ${why}`);
};
const read = (dir, name) => {
  const file = join(root, dir, name);
  return existsSync(file) ? readFileSync(file, "utf8") : null;
};
const definitions = (js) => [...js.matchAll(/customElements\.define\(\s*["']([^"']+)["']/g)].map((m) => m[1]);

if (!root || !existsSync(root) || !gallery || !existsSync(gallery)) {
  for (const id of IDS) fail(id, "missing components root or gallery view");
} else {
  const view = readFileSync(gallery, "utf8");

  // FORM-006
  const otp = read("input-otp", "input-otp.html");
  const otpJs = read("input-otp", "input-otp.js");
  if (otp === null || otpJs === null) fail("FORM-006", "no input-otp view or script");
  else {
    const inputs = otp.match(/<input\b[^>]*>/g) ?? [];
    if (inputs.length !== 1) fail("FORM-006", `the OTP view renders ${inputs.length} inputs, not one`);
    const input = inputs[0] ?? "";
    for (const needle of ['inputmode="numeric"', 'autocomplete="one-time-code"', "pattern=", "maxlength=", "live:model="]) {
      if (!input.includes(needle)) fail("FORM-006", `the OTP input lacks ${needle}`);
    }
    if (!/class="sn-otp-cells" aria-hidden="true"/.test(otp)) fail("FORM-006", "the OTP cells are not hidden from assistive technology");
    const defined = definitions(otpJs);
    if (defined.length !== 1 || defined[0] !== "sn-input-otp") fail("FORM-006", `input-otp.js defines ${defined.join(", ") || "nothing"}`);
    if (/attachShadow|setFormValue|attachInternals/.test(otpJs)) fail("FORM-006", "input-otp.js carries a value or a shadow root; the input is the control");
    if (/input\.value\s*=/.test(otpJs)) fail("FORM-006", "input-otp.js writes the input's value; the control must not depend on it");
    if (!view.includes("::input_otp(")) fail("FORM-006", "the gallery does not mount the input OTP");
  }

  // FORM-007
  const date = read("date-picker", "date-picker.html");
  const dateJs = read("date-picker", "date-picker.js");
  const dateCss = read("date-picker", "date-picker.css");
  if (date === null || dateJs === null || dateCss === null) fail("FORM-007", "no date-picker view, script or stylesheet");
  else {
    const dateInputs = (date.match(/<input\b[^>]*type="date"[^>]*>/g) ?? []);
    if (dateInputs.length !== 1) fail("FORM-007", `the date picker renders ${dateInputs.length} date inputs, not one`);
    if (!/type="date"[^>]*live:model=/.test(date) && !/live:model=[^>]*type="date"/.test(date)) fail("FORM-007", "the date input does not carry the model");
    if (/<input\b(?![^>]*type="(date|radio)")[^>]*>/.test(date)) fail("FORM-007", "the date picker renders an input that is neither the date nor a radio");
    for (const part of ["year", "month", "day"]) {
      const strip = date.match(new RegExp(`<fieldset[^>]*data-sn-part="${part}"[^>]*>[\\s\\S]*?</fieldset>`));
      if (!strip) { fail("FORM-007", `no ${part} strip`); continue; }
      if (!/type="radio"/.test(strip[0])) fail("FORM-007", `the ${part} strip is not a radio group`);
      if (!/<legend/.test(strip[0])) fail("FORM-007", `the ${part} strip has no legend`);
    }
    if (!/scroll-snap-type:\s*x mandatory/.test(dateCss)) fail("FORM-007", "the strips are not scroll-snap containers");
    if (!/scroll-snap-align/.test(dateCss)) fail("FORM-007", "the strip options carry no snap alignment");
    const defined = definitions(dateJs);
    if (defined.length !== 1 || defined[0] !== "sn-date-picker") fail("FORM-007", `date-picker.js defines ${defined.join(", ") || "nothing"}`);
    if (/attachShadow|setFormValue|attachInternals/.test(dateJs)) fail("FORM-007", "date-picker.js carries a value or a shadow root; the input is the control");
    if (!view.includes("::date_picker(")) fail("FORM-007", "the gallery does not mount the date picker");
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
