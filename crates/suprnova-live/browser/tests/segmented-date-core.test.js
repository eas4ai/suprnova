// The developer's core tests are kept intact to pin the date algorithm.
import { test } from "vitest";
import assert from "node:assert/strict";
import * as core from "../../components/segmented-date-picker/date-core.js";

const { bounds: B } = core.createBounds("1900-01-01", "2100-12-31");
const TODAY = { year: 2026, month: 9, day: 23 };
const cands = (seg, parts, b = B) => core.candidatesFor(seg, { ...core.emptyParts(), ...parts }, b);

/** Type a digit string into a segment the way the component does. */
function type(segment, digits, parts = {}, b = B) {
  const c = cands(segment, parts, b);
  let buffer = "";
  const trace = [];
  for (const d of digits) {
    const r = core.processDigit(buffer, d, segment, c);
    trace.push(r.status);
    if (r.status === "commit") return { value: r.value, trace };
    buffer = r.buffer;
  }
  return { buffer, trace };
}

test("leap years and month lengths", () => {
  assert.equal(core.isLeapYear(2024), true);
  assert.equal(core.isLeapYear(1900), false);
  assert.equal(core.isLeapYear(2000), true);
  assert.equal(core.daysInMonth(2025, 2), 28);
  assert.equal(core.daysInMonth(2024, 2), 29);
  assert.equal(core.daysInMonth(2026, 4), 30);
  assert.equal(core.daysInMonth(2026, 1), 31);
});

test("canonical parse/format is strict and timezone-free", () => {
  assert.deepEqual(core.parseISODate("2026-09-23"), TODAY);
  assert.equal(core.parseISODate("2026-02-29"), null);
  assert.equal(core.parseISODate("2026-9-23"), null);
  assert.equal(core.parseISODate("2026-13-01"), null);
  assert.equal(core.parseISODate(""), null);
  assert.equal(core.toCanonical({ year: 987, month: 1, day: 2 }), "0987-01-02");
  assert.equal(core.toCanonical({ year: 2026, month: 9, day: null }), "");
});

test("loose paste parsing respects display order", () => {
  assert.deepEqual(core.parseLooseDate("23/09/2026", "DMY"), TODAY);
  assert.deepEqual(core.parseLooseDate("9.23.2026", "MDY"), TODAY);
  assert.deepEqual(core.parseLooseDate("2026-9-23", "DMY"), TODAY);
  assert.equal(core.parseLooseDate("23/09/26", "DMY"), null, "two-digit years are not supported");
  assert.equal(core.parseLooseDate("31/04/2026", "DMY"), null);
  assert.equal(core.parseLooseDate("hello", "YMD"), null);
});

test("bounds: bad or inverted input falls back and reports", () => {
  const bad = core.createBounds("nope", "2000-01-01");
  assert.equal(bad.errors.length, 1);
  assert.deepEqual({ ...bad.bounds.min }, { ...core.DEFAULT_MIN });
  const inverted = core.createBounds("2030-01-01", "2020-01-01");
  assert.equal(inverted.errors.length, 1);
  assert.deepEqual({ ...inverted.bounds.max }, { ...core.DEFAULT_MAX });
});

test("month auto-commit and pending rules (spec §10)", () => {
  for (const d of ["2", "3", "4", "5", "6", "7", "8", "9"]) {
    assert.equal(type("month", d).value, Number(d), `month ${d}`);
  }
  assert.deepEqual(type("month", "1").trace, ["pending"]);
  assert.equal(type("month", "12").value, 12);
  assert.equal(type("month", "10").value, 10);
  assert.equal(type("month", "09").value, 9);
  assert.deepEqual(type("month", "0").trace, ["pending"]);
});

test("invalid digits are rejected, not clamped (spec §12)", () => {
  const r = type("month", "19");
  assert.deepEqual(r.trace, ["pending", "reject"]);
  assert.equal(r.buffer, "1");
  assert.deepEqual(type("month", "00").trace, ["pending", "reject"]);
  assert.deepEqual(type("year", "3").trace, ["reject"]);
  assert.deepEqual(type("day", "32", { year: 2026, month: 1 }).trace, ["pending", "reject"]);
});

test("day auto-commit depends on month length", () => {
  assert.equal(type("day", "4", { year: 2026, month: 9 }).value, 4);
  assert.deepEqual(type("day", "3", { year: 2026, month: 9 }).trace, ["pending"]);
  assert.equal(type("day", "3", { year: 2025, month: 2 }).value, 3, "Feb has no day 3x");
  assert.equal(type("day", "30", { year: 2026, month: 9 }).value, 30);
  assert.deepEqual(type("day", "31", { year: 2026, month: 9 }).trace, ["pending", "reject"]);
});

test("year needs four digits and respects bounds", () => {
  const r = type("year", "2026");
  assert.equal(r.value, 2026);
  assert.deepEqual(r.trace, ["pending", "pending", "pending", "commit"]);
  assert.deepEqual(type("year", "22").trace, ["pending", "reject"], "22xx is past 2100");
  assert.equal(core.resolveBuffer("20", "year", cands("year")), null);
});

test("buffer resolution on Tab/blur zero-pads shorthand", () => {
  assert.equal(core.resolveBuffer("1", "month", cands("month")), 1);
  assert.equal(core.resolveBuffer("0", "month", cands("month")), null);
  assert.equal(core.resolveBuffer("", "month", cands("month")), null);
  assert.equal(core.resolveBuffer("3", "day", cands("day", { year: 2026, month: 9 })), 3);
  assert.equal(core.padSegment("month", 9), "09");
});

test("dependent day clamps (spec §29)", () => {
  const jan31 = { year: 2026, month: 1, day: 31 };
  const r = core.setSegment(jan31, "month", 4, B);
  assert.deepEqual(r.parts, { year: 2026, month: 4, day: 30 });
  assert.deepEqual(r.adjusted, [{ segment: "day", from: 31, to: 30, restored: false }]);
  const leap = core.setSegment({ year: 2024, month: 2, day: 29 }, "year", 2025, B);
  assert.deepEqual(leap.parts, { year: 2025, month: 2, day: 28 });
});

test("partial dates allow union of possibilities, then clamp", () => {
  assert.equal(cands("day", { month: 2 }).at(-1), 29, "Feb 29 possible while year is empty");
  assert.equal(cands("day", {}).at(-1), 31);
  const r = core.setSegment({ year: null, month: 2, day: 29 }, "year", 2025, B);
  assert.equal(r.parts.day, 28);
  assert.equal(core.setSegment({ year: null, month: null, day: 31 }, "month", 4, B).parts.day, 30);
});

test("min/max restrict candidates and clamp on edits (spec §28)", () => {
  const { bounds } = core.createBounds("2026-03-15", "2027-02-10");
  assert.deepEqual(cands("year", {}, bounds), [2026, 2027]);
  assert.deepEqual(cands("month", { year: 2026 }, bounds), [3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
  assert.deepEqual(cands("month", { year: 2027 }, bounds), [1, 2]);
  assert.deepEqual(cands("month", {}, bounds), [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
  assert.equal(cands("day", { year: 2026, month: 3 }, bounds)[0], 15);
  assert.equal(cands("day", { year: 2027, month: 2 }, bounds).at(-1), 10);
  const r = core.setSegment({ year: 2027, month: 1, day: 20 }, "year", 2026, bounds);
  assert.deepEqual(r.parts, { year: 2026, month: 3, day: 20 });
  assert.equal(core.setSegment({ year: 2026, month: 5, day: 1 }, "year", 2030, bounds), null);
  assert.deepEqual(type("year", "2028", {}, bounds).trace, [
    "pending",
    "pending",
    "pending",
    "reject",
  ]);
});

test("non-contiguous month union across a year boundary", () => {
  const { bounds } = core.createBounds("2025-11-01", "2026-02-28");
  assert.deepEqual(cands("month", {}, bounds), [1, 2, 11, 12]);
});

test("stepping wraps month/day and stops year (spec §19)", () => {
  const step = (parts, seg, d) => core.stepSegment(parts, seg, d, B, TODAY).parts;
  assert.equal(step({ year: 2026, month: 12, day: 1 }, "month", 1).month, 1);
  assert.equal(step({ year: 2026, month: 1, day: 1 }, "month", -1).month, 12);
  assert.equal(step({ year: 2026, month: 9, day: 30 }, "day", 1).day, 1);
  assert.equal(step({ year: 2026, month: 9, day: 1 }, "day", -1).day, 30);
  assert.equal(step({ year: 2100, month: 1, day: 1 }, "year", 1).year, 2100);
  assert.equal(step({ year: 1900, month: 1, day: 1 }, "year", -1).year, 1900);
  assert.equal(step(core.emptyParts(), "month", 1).month, 9, "empty segment takes suggestion");
});

test("small ranges do not wrap", () => {
  const { bounds } = core.createBounds("2026-09-20", "2026-09-24");
  const r = core.stepSegment({ year: 2026, month: 9, day: 24 }, "day", 1, bounds, TODAY);
  assert.equal(r.parts.day, 24);
});

test("suggestion is pulled into bounds", () => {
  const { bounds } = core.createBounds("2030-01-01", "2035-12-31");
  assert.equal(core.suggestionFor("year", core.emptyParts(), bounds, TODAY), 2030);
});

test("sanitizeToBounds clears out-of-range segments top-down", () => {
  const { bounds } = core.createBounds("2027-01-01", "2030-12-31");
  const r = core.sanitizeToBounds({ year: 2026, month: 9, day: 23 }, bounds);
  assert.deepEqual(r.parts, { year: null, month: 9, day: 23 });
  assert.deepEqual(r.cleared, ["year"]);
});

test("locale-derived order", () => {
  assert.equal(core.orderFromLocale("en-US"), "MDY");
  assert.equal(core.orderFromLocale("en-GB"), "DMY");
  assert.equal(core.orderFromLocale("sv-SE"), "YMD");
  assert.equal(core.orderFromLocale("ja-JP"), "YMD");
  assert.equal(core.monthLabels("en-US", "short")[8], "Sep");
});

test("invalid locale tag warns and falls back instead of throwing", () => {
  const warnings = [];
  const original = console.warn;
  console.warn = (msg) => warnings.push(msg);
  try {
    assert.ok(core.ORDERS[core.orderFromLocale("not a locale!!")]);
    assert.equal(core.monthLabels("not a locale!!", "long").length, 12);
  } finally {
    console.warn = original;
  }
  assert.equal(warnings.length, 2);
  assert.match(warnings[0], /not a locale!!/);
});

test("preferred dependent values survive a clamp and come back when valid", () => {
  const pref = { day: 29 };
  const a = core.setSegment({ year: 2024, month: 2, day: 29 }, "year", 2025, B, pref);
  assert.deepEqual(a.parts, { year: 2025, month: 2, day: 28 });
  assert.deepEqual(a.adjusted, [{ segment: "day", from: 29, to: 28, restored: false }]);
  const b = core.setSegment(a.parts, "year", 2028, B, pref);
  assert.deepEqual(b.parts, { year: 2028, month: 2, day: 29 });
  assert.deepEqual(b.adjusted, [{ segment: "day", from: 28, to: 29, restored: true }]);
  // Partial recovery: preferred 31, April allows 30 - an adjustment, not a restore.
  const c = core.setSegment({ year: 2026, month: 2, day: 28 }, "month", 4, B, { day: 31 });
  assert.deepEqual(c.adjusted, [{ segment: "day", from: 28, to: 30, restored: false }]);
  // stepSegment threads the preference through.
  const d = core.stepSegment({ year: 2025, month: 2, day: 28 }, "year", 1, B, TODAY, pref);
  assert.deepEqual(d.parts, { year: 2026, month: 2, day: 28 });
  const e = core.stepSegment({ year: 2026, month: 2, day: 28 }, "month", 1, B, TODAY, { day: 31 });
  assert.deepEqual(e.parts, { year: 2026, month: 3, day: 31 });
  // Without a preference behaviour is unchanged.
  assert.deepEqual(
    core.setSegment({ year: 2025, month: 2, day: 28 }, "year", 2028, B).adjusted,
    [],
  );
});

test("preferred values respect min/max and apply to month too", () => {
  const { bounds } = core.createBounds("2026-03-15", "2027-12-31");
  // Day 10 is below min in March 2026: nearest valid is 15.
  const a = core.setSegment({ year: 2026, month: 4, day: 10 }, "month", 3, bounds, { day: 10 });
  assert.deepEqual(a.parts, { year: 2026, month: 3, day: 15 });
  // Month clamped by the year's bounds, then restored when the year allows it.
  const b = core.setSegment({ year: 2027, month: 1, day: 5 }, "year", 2026, bounds, {
    month: 1,
    day: 5,
  });
  assert.deepEqual(b.parts, { year: 2026, month: 3, day: 15 });
  const c = core.setSegment(b.parts, "year", 2027, bounds, { month: 1, day: 5 });
  assert.deepEqual(c.parts, { year: 2027, month: 1, day: 5 });
  assert.deepEqual(
    c.adjusted.map((x) => [x.segment, x.restored]),
    [
      ["month", true],
      ["day", true],
    ],
  );
});

test("clampToBounds pulls a date to the nearest end of the range", () => {
  const { bounds } = core.createBounds("2020-01-01", "2020-12-31");
  assert.deepEqual(core.clampToBounds({ year: 2026, month: 9, day: 23 }, bounds), {
    year: 2020,
    month: 12,
    day: 31,
  });
  assert.deepEqual(core.clampToBounds({ year: 2019, month: 5, day: 1 }, bounds), {
    year: 2020,
    month: 1,
    day: 1,
  });
  assert.deepEqual(core.clampToBounds({ year: 2020, month: 6, day: 6 }, bounds), {
    year: 2020,
    month: 6,
    day: 6,
  });
});

test("pending previews narrow reachable values without changing committed parts", () => {
  const years = cands("year");
  assert.equal(core.previewFor("1", "year", years, 2024), 1999);
  assert.equal(core.previewFor("198", "year", years, 2024), 1989);
  assert.equal(core.previewFor("20", "year", years, 2024), 2024);
  assert.equal(core.previewFor("1", "month", cands("month"), 9), 1);
  assert.equal(core.previewFor("", "year", years, 2024), null);
  assert.equal(core.isBufferViable("20", "year", years), true);
  assert.equal(core.isBufferViable("22", "year", years), false);
});
