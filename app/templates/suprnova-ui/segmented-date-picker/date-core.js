/**
 * Suprnova Segmented Date Control - core date logic.
 *
 * Pure functions only: no DOM, no clock, no local-time conversion. Dates are
 * explicit { year, month, day } parts (spec §4); any part may be null while the
 * user is mid-entry (spec §31).
 *
 * Segment hierarchy is year → month → day. A segment's valid values depend only
 * on the segments above it; segments below are clamped after an edit (spec §29).
 */

export const SEGMENTS = Object.freeze(["year", "month", "day"]);
export const ORDERS = Object.freeze({
  YMD: Object.freeze(["year", "month", "day"]),
  DMY: Object.freeze(["day", "month", "year"]),
  MDY: Object.freeze(["month", "day", "year"]),
});

export const DEFAULT_MIN = Object.freeze({ year: 1900, month: 1, day: 1 });
export const DEFAULT_MAX = Object.freeze({ year: 2100, month: 12, day: 31 });

/** Month and day wrap (spec §19) only when there are enough values for a loop to read as one. */
export const MIN_WRAP_COUNT = 7;

const WIDTH = Object.freeze({ year: 4, month: 2, day: 2 });
/** Single-digit shorthand ("9" → 09) applies to month and day, never to year (spec §6.1). */
const ALLOWS_SHORTHAND = Object.freeze({ year: false, month: true, day: true });

export function segmentWidth(segment) {
  return WIDTH[segment];
}

export function isLeapYear(year) {
  return (year % 4 === 0 && year % 100 !== 0) || year % 400 === 0;
}

export function daysInMonth(year, month) {
  if (month === 2) return isLeapYear(year) ? 29 : 28;
  return [4, 6, 9, 11].includes(month) ? 30 : 31;
}

export function emptyParts() {
  return { year: null, month: null, day: null };
}

export function isComplete(parts) {
  return parts.year != null && parts.month != null && parts.day != null;
}

function isRealDate(year, month, day) {
  return (
    Number.isInteger(year) &&
    year >= 1 &&
    year <= 9999 &&
    Number.isInteger(month) &&
    month >= 1 &&
    month <= 12 &&
    Number.isInteger(day) &&
    day >= 1 &&
    day <= daysInMonth(year, month)
  );
}

export function padSegment(segment, value) {
  return String(value).padStart(WIDTH[segment], "0");
}

/** Canonical serialized form (spec §4): YYYY-MM-DD, or '' when incomplete. */
export function toCanonical(parts) {
  if (!isComplete(parts)) return "";
  return `${padSegment("year", parts.year)}-${padSegment("month", parts.month)}-${padSegment("day", parts.day)}`;
}

/** Strict canonical parse. Returns null for anything that is not a real YYYY-MM-DD date. */
export function parseISODate(text) {
  if (typeof text !== "string") return null;
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(text.trim());
  if (!m) return null;
  const [year, month, day] = [Number(m[1]), Number(m[2]), Number(m[3])];
  return isRealDate(year, month, day) ? { year, month, day } : null;
}

/**
 * Lenient parse for pasted text: canonical ISO, or three numeric groups in the
 * configured display order separated by / - . or whitespace. Year must be four digits.
 */
export function parseLooseDate(text, order) {
  if (typeof text !== "string") return null;
  const iso = /^\s*(\d{4})-(\d{1,2})-(\d{1,2})\s*$/.exec(text);
  if (iso) {
    const [year, month, day] = [Number(iso[1]), Number(iso[2]), Number(iso[3])];
    return isRealDate(year, month, day) ? { year, month, day } : null;
  }
  const groups = text.trim().split(/[\/\-.\s]+/);
  const segments = ORDERS[order];
  if (!segments || groups.length !== 3 || !groups.every((g) => /^\d+$/.test(g)))
    return null;
  const parts = emptyParts();
  for (let i = 0; i < 3; i++) {
    const seg = segments[i];
    if (seg === "year" ? groups[i].length !== 4 : groups[i].length > 2)
      return null;
    parts[seg] = Number(groups[i]);
  }
  return isRealDate(parts.year, parts.month, parts.day) ? parts : null;
}

export function compareParts(a, b) {
  return a.year - b.year || a.month - b.month || a.day - b.day;
}

/**
 * Resolve min/max strings into bounds. Unparseable input falls back to the
 * defaults; an inverted range is rejected rather than guessed at.
 * Returns { bounds, errors } so the caller decides how to report problems.
 */
export function createBounds(minText, maxText) {
  const errors = [];
  let min = DEFAULT_MIN;
  let max = DEFAULT_MAX;
  if (minText != null && minText !== "") {
    const parsed = parseISODate(minText);
    if (parsed) min = parsed;
    else
      errors.push(
        `min "${minText}" is not a valid YYYY-MM-DD date; using ${toCanonical(DEFAULT_MIN)}`,
      );
  }
  if (maxText != null && maxText !== "") {
    const parsed = parseISODate(maxText);
    if (parsed) max = parsed;
    else
      errors.push(
        `max "${maxText}" is not a valid YYYY-MM-DD date; using ${toCanonical(DEFAULT_MAX)}`,
      );
  }
  if (compareParts(min, max) > 0) {
    errors.push(
      `min ${toCanonical(min)} is after max ${toCanonical(max)}; using defaults`,
    );
    min = DEFAULT_MIN;
    max = DEFAULT_MAX;
  }
  return {
    bounds: Object.freeze({
      min: Object.freeze({ ...min }),
      max: Object.freeze({ ...max }),
    }),
    errors,
  };
}

export function isWithinBounds(parts, bounds) {
  return (
    compareParts(parts, bounds.min) >= 0 && compareParts(parts, bounds.max) <= 0
  );
}

function range(lo, hi) {
  const out = [];
  for (let v = lo; v <= hi; v++) out.push(v);
  return out;
}

function monthRange(year, bounds) {
  if (year < bounds.min.year || year > bounds.max.year) return null;
  const lo = year === bounds.min.year ? bounds.min.month : 1;
  const hi = year === bounds.max.year ? bounds.max.month : 12;
  return [lo, hi];
}

function dayRange(year, month, bounds) {
  const months = monthRange(year, bounds);
  if (!months || month < months[0] || month > months[1]) return null;
  const lo =
    year === bounds.min.year && month === bounds.min.month ? bounds.min.day : 1;
  let hi = daysInMonth(year, month);
  if (year === bounds.max.year && month === bounds.max.month)
    hi = Math.min(hi, bounds.max.day);
  return lo <= hi ? [lo, hi] : null;
}

/**
 * Years that stand in for "any year" when the year segment is empty. Interior
 * years are unconstrained by bounds, so one leap and one common interior year
 * represent all of them exactly.
 */
function representativeYears(bounds) {
  const lo = bounds.min.year;
  const hi = bounds.max.year;
  const years = new Set([lo, hi]);
  let leap = false;
  let common = false;
  // A leap year occurs within any 8 consecutive years (e.g. 1896 → 1904).
  for (
    let y = lo + 1;
    y <= Math.min(lo + 8, hi - 1) && !(leap && common);
    y++
  ) {
    if (isLeapYear(y) && !leap) {
      years.add(y);
      leap = true;
    }
    if (!isLeapYear(y) && !common) {
      years.add(y);
      common = true;
    }
  }
  return [...years];
}

const yearCache = new WeakMap();

/**
 * All values the segment may take given the segments above it. With a higher
 * segment empty, this is the union over every value it could still take.
 * Result is sorted ascending and must not be mutated.
 */
export function candidatesFor(segment, parts, bounds) {
  if (segment === "year") {
    let years = yearCache.get(bounds);
    if (!years) {
      years = Object.freeze(range(bounds.min.year, bounds.max.year));
      yearCache.set(bounds, years);
    }
    return years;
  }
  const years = parts.year != null ? [parts.year] : representativeYears(bounds);
  const values = new Set();
  if (segment === "month") {
    for (const y of years) {
      const r = monthRange(y, bounds);
      if (r) for (let m = r[0]; m <= r[1]; m++) values.add(m);
    }
  } else if (segment === "day") {
    const months = parts.month != null ? [parts.month] : range(1, 12);
    for (const y of years) {
      for (const m of months) {
        const r = dayRange(y, m, bounds);
        if (r) for (let d = r[0]; d <= r[1]; d++) values.add(d);
      }
    }
  } else {
    throw new RangeError(`Unknown segment "${segment}"`);
  }
  return [...values].sort((a, b) => a - b);
}

export function indexOfValue(candidates, value) {
  let lo = 0;
  let hi = candidates.length - 1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (candidates[mid] === value) return mid;
    if (candidates[mid] < value) lo = mid + 1;
    else hi = mid - 1;
  }
  return -1;
}

/** Nearest candidate by numeric distance; ties resolve downward. */
export function nearestValue(candidates, value) {
  if (candidates.length === 0) return null;
  let best = candidates[0];
  for (const c of candidates) {
    if (Math.abs(c - value) < Math.abs(best - value)) best = c;
  }
  return best;
}

export function segmentWraps(segment, count) {
  return segment !== "year" && count >= MIN_WRAP_COUNT;
}

/**
 * Set one segment and resolve every lower, non-empty segment into its new valid
 * range (spec §29). Returns null when the value is not valid for the segment.
 *
 * `preferred` optionally carries the value the user last chose for a lower
 * segment ({ month, day }). A dependent segment resolves to the valid value
 * nearest its preferred value (else its current value), so an intent that was
 * clamped away (Feb 29 → 28 in a common year) comes back when it fits again.
 *
 * `adjusted` lists each dependent segment that changed, for announcement;
 * `restored` marks a change that landed exactly on the preferred value.
 */
export function setSegment(parts, segment, value, bounds, preferred = null) {
  if (indexOfValue(candidatesFor(segment, parts, bounds), value) < 0)
    return null;
  const next = { ...parts, [segment]: value };
  const adjusted = [];
  for (const lower of SEGMENTS.slice(SEGMENTS.indexOf(segment) + 1)) {
    const from = next[lower];
    if (from == null) continue;
    const allowed = candidatesFor(lower, next, bounds);
    const want = preferred?.[lower] ?? from;
    const to =
      indexOfValue(allowed, want) >= 0 ? want : nearestValue(allowed, want);
    if (to !== from) {
      adjusted.push({
        segment: lower,
        from,
        to,
        restored: to === preferred?.[lower],
      });
      next[lower] = to;
    }
  }
  return { parts: next, adjusted };
}

/** A complete date pulled to the nearest end of the range when it falls outside. */
export function clampToBounds(parts, bounds) {
  if (compareParts(parts, bounds.min) < 0) return { ...bounds.min };
  if (compareParts(parts, bounds.max) > 0) return { ...bounds.max };
  return { ...parts };
}

/** The value an empty segment starts from: `reference` (normally today) pulled into range. */
export function suggestionFor(segment, parts, bounds, reference) {
  return nearestValue(
    candidatesFor(segment, parts, bounds),
    reference[segment],
  );
}

/**
 * Move a segment by `delta` steps. Month/day wrap; year stops at its bounds.
 * An empty segment takes the suggestion instead of stepping.
 */
export function stepSegment(
  parts,
  segment,
  delta,
  bounds,
  reference,
  preferred = null,
) {
  const candidates = candidatesFor(segment, parts, bounds);
  const current = parts[segment];
  let value;
  if (current == null) {
    value = suggestionFor(segment, parts, bounds, reference);
  } else {
    const index = indexOfValue(candidates, current);
    if (index < 0) {
      value = nearestValue(candidates, current);
    } else if (segmentWraps(segment, candidates.length)) {
      const n = candidates.length;
      value = candidates[(((index + delta) % n) + n) % n];
    } else {
      value =
        candidates[Math.max(0, Math.min(candidates.length - 1, index + delta))];
    }
  }
  return setSegment(parts, segment, value, bounds, preferred);
}

export function segmentBoundary(parts, segment, bounds, which) {
  const candidates = candidatesFor(segment, parts, bounds);
  return which === "first" ? candidates[0] : candidates[candidates.length - 1];
}

/**
 * Feed one typed digit into a segment's edit buffer (spec §9-§12).
 *   reject  - the digit cannot lead to a valid value; buffer unchanged.
 *   pending - valid so far and could still grow ("1" as a month).
 *   commit  - no valid longer entry exists; `value` is final ("4" → April).
 */
export function processDigit(buffer, digit, segment, candidates) {
  const width = WIDTH[segment];
  const next = buffer + digit;
  if (!/^\d$/.test(digit) || next.length > width)
    return { status: "reject", buffer };
  const numeric = Number(next);
  const isPrefix = candidates.some((c) =>
    padSegment(segment, c).startsWith(next),
  );
  const isShorthand =
    ALLOWS_SHORTHAND[segment] &&
    next.length < width &&
    numeric !== 0 &&
    indexOfValue(candidates, numeric) >= 0;
  if (!isPrefix && !isShorthand) return { status: "reject", buffer };
  const canGrow = isPrefix && next.length < width;
  if (canGrow) return { status: "pending", buffer: next };
  return { status: "commit", buffer: "", value: numeric };
}

/** Resolve a pending buffer on Tab/Enter/blur/separator. Null when it is not a complete value. */
export function resolveBuffer(buffer, segment, candidates) {
  if (!buffer) return null;
  const value = Number(buffer);
  if (buffer.length === WIDTH[segment])
    return indexOfValue(candidates, value) >= 0 ? value : null;
  if (
    ALLOWS_SHORTHAND[segment] &&
    value !== 0 &&
    indexOfValue(candidates, value) >= 0
  )
    return value;
  return null;
}

/** Whether a pending buffer can still become a valid value (after candidates changed). */
export function isBufferViable(buffer, segment, candidates) {
  if (!buffer) return false;
  return (
    resolveBuffer(buffer, segment, candidates) != null ||
    candidates.some((c) => padSegment(segment, c).startsWith(buffer))
  );
}

/**
 * Where the scroller points while `buffer` is pending. Display only: the
 * committed value is never touched. Policy: what resolveBuffer would commit if
 * the user stopped now (month "1" → January); otherwise the reachable value
 * nearest `anchor` (year "198_" from 2024 → 1989), so the strip narrows in.
 * Null for an empty buffer or one with no reachable value.
 */
export function previewFor(buffer, segment, candidates, anchor) {
  if (!buffer) return null;
  const now = resolveBuffer(buffer, segment, candidates);
  if (now != null) return now;
  const reachable = candidates.filter((c) =>
    padSegment(segment, c).startsWith(buffer),
  );
  return nearestValue(reachable, anchor);
}

/**
 * Clear, top-down, any segment no longer valid under new bounds. Used when
 * min/max change underneath an existing value; user edits clamp instead.
 */
export function sanitizeToBounds(parts, bounds) {
  const next = { ...parts };
  const cleared = [];
  for (const segment of SEGMENTS) {
    if (next[segment] == null) continue;
    if (indexOfValue(candidatesFor(segment, next, bounds), next[segment]) < 0) {
      cleared.push(segment);
      next[segment] = null;
    }
  }
  return { parts: next, cleared };
}

/**
 * Intl formatter for a developer-supplied locale. An invalid locale tag is a configuration
 * mistake, not a user error: warn with the offending tag and fall back to the runtime default.
 */
function dateFormat(locale, options) {
  try {
    return new Intl.DateTimeFormat(locale, { ...options, timeZone: "UTC" });
  } catch (error) {
    if (!(error instanceof RangeError)) throw error;
    console.warn(
      `<sn-segmented-date>: locale "${locale}" is not a valid language tag (${error.message}); using the default locale`,
    );
    return new Intl.DateTimeFormat(undefined, { ...options, timeZone: "UTC" });
  }
}

/** Display order implied by a locale's numeric date format; YMD when it cannot be determined. */
export function orderFromLocale(locale) {
  const key = dateFormat(locale, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  })
    .formatToParts(Date.UTC(2026, 8, 23))
    .filter((p) => p.type === "year" || p.type === "month" || p.type === "day")
    .map((p) => p.type[0].toUpperCase())
    .join("");
  return key in ORDERS ? key : "YMD";
}

/** Twelve month labels for the scroller; `style` is 'short' or 'long'. */
export function monthLabels(locale, style) {
  const fmt = dateFormat(locale, { month: style });
  return range(0, 11).map((m) => fmt.format(Date.UTC(2026, m, 15)));
}
