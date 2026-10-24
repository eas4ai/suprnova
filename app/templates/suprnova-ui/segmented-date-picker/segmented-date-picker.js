/**
 * <sn-segmented-date> - Suprnova Segmented Date Control, v1.
 *
 * One form-associated element; three spinbutton segments; one contextual
 * scroller. `#parts` is the single source of truth (spec §2.4). Typing and the
 * scroller both write to it; the segments and the scroller only render it.
 */
import * as core from "./date-core.js";
import { ValueScroller } from "./value-scroller.js";

const LABELS = Object.freeze({ year: "Year", month: "Month", day: "Day" });
const PLACEHOLDERS = Object.freeze({ year: "YYYY", month: "MM", day: "DD" });
const EXTRA_SEPARATORS = new Set(["/", "-", ".", " "]);
const MONTH_DISPLAYS = new Set(["numeric", "short", "long"]);
const ITEM_WIDTH = Object.freeze({
  year: 64,
  month: 48,
  day: 48,
  short: 64,
  long: 112,
});
const REJECT_MS = 320;
const ADJUST_MS = 640;

const reducedMotionQuery =
  typeof matchMedia === "function"
    ? matchMedia("(prefers-reduced-motion: reduce)")
    : { matches: false };

function todayParts() {
  // Local calendar date is the right meaning for "suggest today"; it is never the stored value.
  const d = new Date();
  return { year: d.getFullYear(), month: d.getMonth() + 1, day: d.getDate() };
}

/** ElementInternals ARIA reflection (role, ariaDisabled); absent in older engines. */
const INTERNALS_ARIA =
  typeof ElementInternals === "function" &&
  "role" in ElementInternals.prototype;

function readBooleanDefaultTrue(el, name) {
  const v = el.getAttribute(name);
  return v == null || v.toLowerCase() !== "false";
}

/** Keeps the segments, scroller, form value and Live bridge on one date state. */
export class SegmentedDatePicker extends HTMLElement {
  static formAssociated = true;
  static observedAttributes = [
    "value",
    "min",
    "max",
    "order",
    "separator",
    "month-display",
    "locale",
    "disabled",
    "readonly",
    "required",
    "scroller",
    "motion",
    "default-today",
  ];

  #internals;
  #root;
  #bridge;
  #observer;
  #frame = 0;
  #initialized = false;
  #invalidValue = null;
  #initialValue = null;
  #field;
  #inputs = {};
  #separators = [];
  #live;
  #scroller;

  #parts = core.emptyParts();
  #bounds = core.createBounds(null, null).bounds;
  #order = ["year", "month", "day"];
  #buffer = "";
  #active = null;
  #hasFocus = false;
  #valueAtFocus = "";
  #dirty = false;
  #formDisabled = false;
  #swallowSeparator = false;
  #composing = false;
  #monthLabelsCache = new Map();
  #rejectTimers = new Map();
  #adjustTimers = new Map();
  /** Month/day the user last chose during this focus session; see core.setSegment. */
  #preferred = {};

  constructor() {
    super();
    this.#internals = this.attachInternals();
    if (INTERNALS_ARIA) this.#internals.role = "group";
  }

  connectedCallback() {
    if (this.#initialized) {
      this.#observeBridge();
      return;
    }
    this.#root = this.querySelector("[data-sn-editor]");
    this.#bridge = this.querySelector("[data-sn-value]");
    if (!this.#root || !this.#bridge) {
      // When the definition predates parser insertion, children arrive after
      // connectedCallback. Upgrade them once parsing finishes.
      queueMicrotask(() => {
        if (!this.isConnected || this.#initialized) return;
        this.#ensureMarkup();
        this.connectedCallback();
      });
      return;
    }
    this.#initialized = true;
    this.#field = this.#root.querySelector(".sn-segmented-field");
    this.#live = this.#root.querySelector('[role="status"]');

    for (const segment of core.SEGMENTS)
      this.#inputs[segment] = this.#createInput(segment);
    this.#scroller = new ValueScroller(this.#root, {
      onSelect: (value) => this.#selectFromScroller(value),
      onStep: (delta) => this.#step(delta),
      getAnchor: () => ({
        field: this.#field.getBoundingClientRect(),
        segment: (
          this.#inputs[this.#active] ?? this.#field
        ).getBoundingClientRect(),
      }),
      motionEnabled: () =>
        this.motion !== "none" && !reducedMotionQuery.matches,
    });

    this.#root.addEventListener("focusin", (e) => this.#onFocusIn(e));
    this.#root.addEventListener("focusout", (e) => this.#onFocusOut(e));
    this.#field.addEventListener("pointerdown", (e) =>
      this.#onFieldPointerDown(e),
    );

    this.addEventListener("click", (event) => {
      if (event.target === this) this.focus();
    });
    this.#applyBounds();
    this.#setFromString(
      this.#initialValue ?? this.#defaultString(),
      "initial value",
    );
    this.#initialValue = null;
    this.#applyInteractivity();
    this.#observeBridge();

    this.#applyOrder();
    this.#renderAll();
    this.#updateFormValue();

    // Without ElementInternals ARIA reflection, fall back to host attributes.
    if (!INTERNALS_ARIA && !this.hasAttribute("role"))
      this.setAttribute("role", "group");
    // Locale-derived order can depend on an ancestor's lang, known only once connected.
    if (!this.hasAttribute("order")) {
      this.#applyOrder();
      this.#renderAll();
    }
    // Upgrade any property set before the element was defined.
    for (const prop of [
      "value",
      "min",
      "max",
      "order",
      "disabled",
      "readOnly",
      "required",
      "defaultToday",
    ]) {
      if (Object.prototype.hasOwnProperty.call(this, prop)) {
        const v = this[prop];
        delete this[prop];
        this[prop] = v;
      }
    }
  }

  disconnectedCallback() {
    if (!this.#initialized) return;
    this.#observer?.disconnect();
    cancelAnimationFrame(this.#frame);
    this.#frame = 0;
    this.#leave();
    for (const [segment, timer] of this.#rejectTimers) {
      clearTimeout(timer);
      this.#inputs[segment].removeAttribute("data-sn-rejected");
    }
    this.#rejectTimers.clear();
    for (const [segment, t] of this.#adjustTimers) {
      clearTimeout(t);
      this.#clearAdjusted(segment);
    }
    this.#adjustTimers.clear();
  }

  attributeChangedCallback(name, oldValue, newValue) {
    if (oldValue === newValue || !this.#initialized) return;
    switch (name) {
      case "value":
      case "default-today":
        if (!this.#dirty)
          this.#setFromString(
            this.#defaultString(),
            name === "value" ? "value attribute" : "default",
          );
        break;
      case "min":
      case "max":
        this.#applyBounds();
        break;
      case "order":
      case "locale":
        this.#monthLabelsCache.clear();
        this.#applyOrder();
        this.#renderAll();
        this.#syncScroller("none");
        break;
      case "separator":
        for (const s of this.#separators) s.textContent = this.separator;
        break;
      case "month-display":
        this.#syncScroller("none");
        break;
      case "disabled":
      case "readonly":
        this.#applyInteractivity();
        break;
      case "required":
        this.#updateFormValue();
        break;
      case "scroller":
        if (!this.scroller) this.#closeScroller();
        else if (this.#active) this.#openScroller();
        break;
      case "motion":
        break;
    }
  }

  #ensureMarkup() {
    // The macro renders this content for SSR. A directly created element
    // builds the same light-DOM editor after parser children have arrived.
    if (!this.querySelector("[data-sn-value]")) {
      const bridge = document.createElement("input");
      bridge.type = "text";
      bridge.hidden = true;
      bridge.setAttribute("data-sn-value", "");
      this.prepend(bridge);
    }
    if (this.querySelector("[data-sn-editor]")) return;
    const editor = document.createElement("div");
    editor.setAttribute("data-sn-editor", "");
    editor.innerHTML = `<div class="sn-segmented-field">${core.SEGMENTS.map((segment) => `<span data-segment="${segment}"></span>`).join("")}</div><span class="sn-segmented-sr-only" role="status" aria-live="polite"></span>`;
    this.append(editor);
  }

  // Moving a persistent element keeps its existing listeners (UI-020).
  connectedMoveCallback() {}

  #observeBridge() {
    this.#observer?.disconnect();
    this.#observer = new MutationObserver(() => {
      if (this.#frame) return;
      // Live restores unsent native input values after morphing. Read after
      // that restoration, including an authoritative server correction.
      this.#frame = requestAnimationFrame(() => {
        this.#frame = 0;
        if (this.isConnected && this.#bridge.value !== this.value) {
          this.#setFromString(this.#bridge.value, "live:model");
        }
      });
    });
    this.#observer.observe(this.#bridge, {
      attributes: true,
      attributeFilter: ["value", "data-suprnova-live-authoritative"],
    });
  }

  /** A label or application focus enters the first segment in display order. */
  focus(options) {
    if (!this.disabled && !this.#formDisabled)
      this.focusSegment(this.#order[0], options);
  }

  // ---- public API ---------------------------------------------------------------

  /** Canonical YYYY-MM-DD, or '' while any segment is empty (spec §31-§32). */
  get value() {
    return core.toCanonical(this.#parts);
  }
  set value(v) {
    this.#dirty = true;
    this.#setFromString(v == null ? "" : String(v), "value");
  }

  get defaultValue() {
    return this.getAttribute("value") ?? "";
  }
  set defaultValue(v) {
    this.setAttribute("value", v);
  }

  /** Copy of the committed parts, including partial entry. */
  get valueAsParts() {
    return { ...this.#parts };
  }

  get min() {
    return this.getAttribute("min") ?? "";
  }
  set min(v) {
    v ? this.setAttribute("min", v) : this.removeAttribute("min");
  }
  get max() {
    return this.getAttribute("max") ?? "";
  }
  set max(v) {
    v ? this.setAttribute("max", v) : this.removeAttribute("max");
  }

  get order() {
    return this.#order.map((s) => s[0].toUpperCase()).join("");
  }
  set order(v) {
    v ? this.setAttribute("order", v) : this.removeAttribute("order");
  }

  get separator() {
    const s = this.getAttribute("separator");
    return s && s.length <= 3 ? s : "/";
  }
  set separator(v) {
    this.setAttribute("separator", v);
  }

  get monthDisplay() {
    const v = this.getAttribute("month-display");
    return MONTH_DISPLAYS.has(v) ? v : "numeric";
  }
  set monthDisplay(v) {
    this.setAttribute("month-display", v);
  }

  get locale() {
    return (
      this.getAttribute("locale") ||
      this.closest("[lang]")?.lang ||
      navigator.language ||
      "en"
    );
  }
  set locale(v) {
    v ? this.setAttribute("locale", v) : this.removeAttribute("locale");
  }

  get autoAdvance() {
    return readBooleanDefaultTrue(this, "auto-advance");
  }
  set autoAdvance(v) {
    v
      ? this.removeAttribute("auto-advance")
      : this.setAttribute("auto-advance", "false");
  }

  /** When no value attribute is given, the default (initial and reset) value is today, clamped into min/max. */
  get defaultToday() {
    return this.hasAttribute("default-today");
  }
  set defaultToday(v) {
    this.toggleAttribute("default-today", Boolean(v));
  }

  get scroller() {
    return readBooleanDefaultTrue(this, "scroller");
  }
  set scroller(v) {
    v
      ? this.removeAttribute("scroller")
      : this.setAttribute("scroller", "false");
  }

  get motion() {
    return this.getAttribute("motion") === "none" ? "none" : "subtle";
  }
  set motion(v) {
    this.setAttribute("motion", v === "none" ? "none" : "subtle");
  }

  get disabled() {
    return this.hasAttribute("disabled");
  }
  set disabled(v) {
    this.toggleAttribute("disabled", Boolean(v));
  }
  get readOnly() {
    return this.hasAttribute("readonly");
  }
  set readOnly(v) {
    this.toggleAttribute("readonly", Boolean(v));
  }
  get required() {
    return this.hasAttribute("required");
  }
  set required(v) {
    this.toggleAttribute("required", Boolean(v));
  }

  get name() {
    return this.getAttribute("name") ?? "";
  }
  set name(v) {
    this.setAttribute("name", v);
  }
  get type() {
    return "sn-date";
  }
  get form() {
    return this.#internals.form;
  }
  get labels() {
    return this.#internals.labels;
  }
  get validity() {
    return this.#internals.validity;
  }
  get validationMessage() {
    return this.#internals.validationMessage;
  }
  get willValidate() {
    return this.#internals.willValidate;
  }
  checkValidity() {
    return this.#internals.checkValidity();
  }
  reportValidity() {
    return this.#internals.reportValidity();
  }

  /** Focus a specific segment ('year' | 'month' | 'day'). */
  focusSegment(segment, options) {
    this.#inputs[segment]?.focus(options);
  }

  clear() {
    this.#dirty = true;
    this.#setFromString("", "clear");
  }

  // ---- form lifecycle -------------------------------------------------------------

  formResetCallback() {
    this.#dirty = false;
    this.#setFromString(this.#defaultString(), "reset");
  }

  formStateRestoreCallback(state) {
    if (typeof state !== "string") return;
    // A restored value was the user's (native inputs restore as dirty), so the
    // default - notably default-today - must not re-derive over it later.
    this.#dirty = true;
    this.#setFromString(state, "restore");
  }

  formDisabledCallback(disabled) {
    this.#formDisabled = disabled;
    if (this.#initialized) this.#applyInteractivity();
  }

  // ---- construction / configuration ----------------------------------------------

  #createInput(segment) {
    const input = this.#root.querySelector(`[data-segment="${segment}"]`);
    input.className = "sn-segmented-segment";
    input.tabIndex = 0;
    input.contentEditable = "true";
    input.inputMode = "numeric";
    input.spellcheck = false;
    input.setAttribute("autocorrect", "off");
    input.setAttribute("autocapitalize", "off");
    input.setAttribute("role", "spinbutton");
    input.setAttribute("aria-label", LABELS[segment]);
    input.setAttribute("aria-placeholder", PLACEHOLDERS[segment]);
    input.setAttribute("part", `segment ${segment}`);
    input.dataset.placeholder = PLACEHOLDERS[segment];
    input.dataset.segment = segment;

    input.addEventListener("keydown", (e) => this.#onKeyDown(e, segment));
    input.addEventListener("beforeinput", (e) =>
      this.#onBeforeInput(e, segment),
    );
    input.addEventListener("input", (e) => this.#onNativeInput(e, segment));
    input.addEventListener("paste", (e) => this.#onPaste(e, segment));
    input.addEventListener("drop", (e) => e.preventDefault());
    input.addEventListener("compositionstart", () => {
      this.#composing = true;
    });
    input.addEventListener("compositionend", (e) => {
      this.#composing = false;
      if (this.#editable && e.data) this.#typeText(e.data);
      this.#renderSegment(this.#active ?? segment, true);
    });
    input.addEventListener("pointerdown", () => {
      if (this.#active === segment) this.#openScroller();
    });
    return input;
  }

  #applyOrder() {
    const requested = (this.getAttribute("order") || "").toUpperCase();
    const key =
      requested in core.ORDERS ? requested : core.orderFromLocale(this.locale);
    this.#order = [...core.ORDERS[key]];
    const focused = document.activeElement;
    this.#field.replaceChildren();
    this.#separators = [];
    this.#order.forEach((segment, i) => {
      if (i > 0) {
        const sep = document.createElement("span");
        sep.className = "sn-segmented-separator";
        sep.setAttribute("part", "separator");
        sep.setAttribute("aria-hidden", "true");
        sep.textContent = this.separator;
        this.#separators.push(sep);
        this.#field.append(sep);
      }
      const input = this.#inputs[segment];
      input.enterKeyHint = i === this.#order.length - 1 ? "done" : "next";
      this.#field.append(input);
    });
    if (focused && document.activeElement !== focused) focused.focus();
  }

  #applyBounds() {
    const { bounds, errors } = core.createBounds(
      this.getAttribute("min"),
      this.getAttribute("max"),
    );
    for (const e of errors) console.warn(`<sn-segmented-date>: ${e}`);
    const buffer = this.#buffer;
    this.#bounds = bounds;
    this.#invalidValue =
      core.isComplete(this.#parts) && !core.isWithinBounds(this.#parts, bounds)
        ? this.#parts
        : null;
    if (!this.#dirty && this.defaultToday && !this.hasAttribute("value")) {
      // Untouched default of "today": re-clamp into the new range rather than clearing it.
      this.#setFromString(this.#defaultString(), "default");
      this.#reconcileBuffer(buffer);
      this.#syncScroller("none");
      return;
    }
    const { parts, cleared } = core.sanitizeToBounds(this.#parts, bounds);
    if (cleared.length) {
      console.warn(
        `<sn-segmented-date>: cleared ${cleared.join(", ")} outside the new min/max range`,
      );
    }
    this.#parts = parts;
    this.#buffer = "";
    this.#renderAll();
    this.#updateFormValue();
    this.#reconcileBuffer(buffer);
    this.#syncScroller("none");
  }

  #reconcileBuffer(buffer) {
    const segment = this.#active;
    if (!buffer || !segment) return;
    if (core.isBufferViable(buffer, segment, this.#candidates(segment))) {
      this.#buffer = buffer;
    } else {
      this.#buffer = "";
      this.#announce(`Incomplete ${LABELS[segment].toLowerCase()} discarded`);
    }
    this.#renderSegment(segment);
  }

  #applyInteractivity() {
    const disabled = this.disabled || this.#formDisabled;
    for (const input of Object.values(this.#inputs)) {
      if (disabled) {
        input.removeAttribute("tabindex");
        if (input.matches(":focus")) input.blur();
      } else {
        input.tabIndex = 0;
      }
      input.contentEditable = disabled || this.readOnly ? "false" : "true";
      input.setAttribute("aria-disabled", String(disabled));
      input.setAttribute("aria-readonly", String(this.readOnly));
    }
    this.#field.setAttribute("aria-disabled", String(disabled));
    this.#field.setAttribute("aria-readonly", String(this.readOnly));
    this.#bridge.disabled = disabled;
    if (INTERNALS_ARIA) this.#internals.ariaDisabled = disabled ? "true" : null;
    else if (disabled) this.setAttribute("aria-disabled", "true");
    else this.removeAttribute("aria-disabled");
    if (disabled || this.readOnly) this.#closeScroller();
    this.#renderAll();
  }

  get #editable() {
    return !this.disabled && !this.#formDisabled && !this.readOnly;
  }

  /** The value the control starts from and resets to. */
  #defaultString() {
    const attr = this.getAttribute("value");
    if (attr != null) return attr;
    if (!this.defaultToday) return "";
    return core.toCanonical(core.clampToBounds(todayParts(), this.#bounds));
  }

  /** Programmatic writes: strict canonical, within bounds, no events (like a native input). */
  #setFromString(text, source) {
    if (!this.#initialized) {
      // Properties may be assigned on a detached, already-defined element.
      this.#initialValue = text;
      const parsed = core.parseISODate(text);
      const bounds = core.createBounds(this.min, this.max).bounds;
      this.#parts =
        parsed && core.isWithinBounds(parsed, bounds)
          ? parsed
          : core.emptyParts();
      return;
    }
    let parts = core.emptyParts();
    this.#invalidValue = null;
    if (text !== "") {
      const parsed = core.parseISODate(text);
      if (!parsed) {
        console.warn(
          `<sn-segmented-date>: ${source} "${text}" is not a valid YYYY-MM-DD date; field cleared`,
        );
      } else if (!core.isWithinBounds(parsed, this.#bounds)) {
        this.#invalidValue = parsed;
        console.warn(
          `<sn-segmented-date>: ${source} "${text}" is outside min/max; field cleared`,
        );
      } else {
        parts = parsed;
      }
    }
    this.#parts = parts;
    this.#buffer = "";
    if (this.#hasFocus) {
      this.#valueAtFocus = this.value;
      this.#seedPreferred();
    }
    this.#renderAll();
    this.#updateFormValue();
    this.#syncScroller("jump");
  }

  // ---- rendering ------------------------------------------------------------------

  #candidates(segment) {
    return core.candidatesFor(segment, this.#parts, this.#bounds);
  }

  #monthLabels(style) {
    const key = `${this.locale}|${style}`;
    if (!this.#monthLabelsCache.has(key))
      this.#monthLabelsCache.set(key, core.monthLabels(this.locale, style));
    return this.#monthLabelsCache.get(key);
  }

  #renderAll() {
    for (const segment of core.SEGMENTS) this.#renderSegment(segment);
  }

  #renderSegment(segment, force = false) {
    const input = this.#inputs[segment];
    const value = this.#parts[segment];
    const editing = segment === this.#active && this.#buffer !== "";
    const width = core.segmentWidth(segment);
    const text = editing
      ? this.#buffer + "_".repeat(width - this.#buffer.length)
      : value == null
        ? ""
        : core.padSegment(segment, value);
    if (force || input.textContent !== text) input.textContent = text;

    const candidates = this.#candidates(segment);
    if (candidates.length) {
      input.setAttribute("aria-valuemin", String(candidates[0]));
      input.setAttribute(
        "aria-valuemax",
        String(candidates[candidates.length - 1]),
      );
    }
    if (editing) {
      input.removeAttribute("aria-valuenow");
      input.setAttribute("aria-valuetext", `${this.#buffer}, typing`);
    } else if (value == null) {
      input.removeAttribute("aria-valuenow");
      input.setAttribute("aria-valuetext", "Empty");
    } else {
      input.setAttribute("aria-valuenow", String(value));
      input.setAttribute("aria-valuetext", this.#valueText(segment, value));
    }
  }

  /** Spec §35: "Month, 9 of 12", "Day, 23 of 30". The label supplies the segment name. */
  #valueText(segment, value) {
    if (segment === "year") return String(value);
    const max =
      segment === "month"
        ? 12
        : this.#parts.year != null && this.#parts.month != null
          ? core.daysInMonth(this.#parts.year, this.#parts.month)
          : this.#candidates("day").at(-1);
    const base = `${value} of ${max}`;
    return segment === "month"
      ? `${this.#monthLabels("long")[value - 1]}, ${base}`
      : base;
  }

  #announce(message) {
    // Clear first so an identical message is announced again.
    this.#live.textContent = "";
    requestAnimationFrame(() => {
      this.#live.textContent = message;
    });
  }

  #reject(segment, message) {
    const input = this.#inputs[segment];
    input.removeAttribute("data-sn-rejected");
    void input.offsetWidth;
    input.setAttribute("data-sn-rejected", "");
    clearTimeout(this.#rejectTimers.get(segment));
    this.#rejectTimers.set(
      segment,
      setTimeout(() => input.removeAttribute("data-sn-rejected"), REJECT_MS),
    );
    this.#announce(message);
  }

  #updateFormValue() {
    const value = this.value;
    this.#internals.setFormValue(value || null, value);
    this.#bridge.value = value;
    const filled = core.SEGMENTS.filter((s) => this.#parts[s] != null);
    const firstEmpty =
      this.#order.find((s) => this.#parts[s] == null) ?? this.#order[0];
    const anchor = this.#inputs[firstEmpty];
    if (this.#invalidValue) {
      const under = core.compareParts(this.#invalidValue, this.#bounds.min) < 0;
      this.#internals.setValidity(
        under ? { rangeUnderflow: true } : { rangeOverflow: true },
        under
          ? `Enter a date on or after ${core.toCanonical(this.#bounds.min)}.`
          : `Enter a date on or before ${core.toCanonical(this.#bounds.max)}.`,
        anchor,
      );
    } else if (filled.length === 0 && this.required) {
      this.#internals.setValidity(
        { valueMissing: true },
        "Enter a date.",
        anchor,
      );
    } else if (filled.length > 0 && filled.length < 3) {
      this.#internals.setValidity(
        { badInput: true },
        `Enter the ${LABELS[firstEmpty].toLowerCase()} to complete the date.`,
        anchor,
      );
    } else {
      this.#internals.setValidity({});
    }
    this.setAttribute("aria-invalid", String(!this.#internals.validity.valid));
    for (const input of Object.values(this.#inputs)) {
      input.setAttribute("aria-required", String(this.required));
    }
  }

  // ---- state transitions -------------------------------------------------------------

  /** Commit a change produced by core.setSegment/stepSegment. */
  #apply(result, motion, segment) {
    if (!result) return;
    const before = this.value;
    this.#invalidValue = null;
    this.#parts = result.parts;
    this.#buffer = "";
    // An explicit choice becomes the intent that later clamps try to preserve.
    if (segment !== "year" && result.parts[segment] != null)
      this.#preferred[segment] = result.parts[segment];
    this.#renderAll();
    if (result.adjusted.length) {
      // One message: a second announce() in the same frame would replace the first.
      this.#announce(
        result.adjusted
          .map(
            ({ segment: s, to, restored }) =>
              `${LABELS[s]} ${restored ? "restored" : "adjusted"} to ${to}`,
          )
          .join(". "),
      );
      for (const { segment: s } of result.adjusted) this.#flashAdjusted(s);
    }
    this.#updateFormValue();
    this.#syncScroller(motion);
    this.#userChanged(before);
  }

  /** After a user edit: mark dirty (value attribute no longer drives the value) and emit input. */
  #userChanged(before) {
    if (this.value === before) return;
    this.#dirty = true;
    this.#emit("input");
  }

  #seedPreferred() {
    this.#preferred = {};
    for (const s of ["month", "day"])
      if (this.#parts[s] != null) this.#preferred[s] = this.#parts[s];
  }

  /** Brief highlight on a segment changed as a consequence of another (spec §29). */
  #flashAdjusted(segment) {
    const input = this.#inputs[segment];
    input.removeAttribute("data-sn-adjusted");
    void input.offsetWidth; // restart the animation on back-to-back adjustments
    input.setAttribute("data-sn-adjusted", "");
    input.part.add("adjusted");
    clearTimeout(this.#adjustTimers.get(segment));
    this.#adjustTimers.set(
      segment,
      setTimeout(() => {
        this.#adjustTimers.delete(segment);
        this.#clearAdjusted(segment);
      }, ADJUST_MS),
    );
  }

  #clearAdjusted(segment) {
    const input = this.#inputs[segment];
    input.removeAttribute("data-sn-adjusted");
    input.part.remove("adjusted");
  }

  #commitValue(segment, value, motion) {
    const result = core.setSegment(
      this.#parts,
      segment,
      value,
      this.#bounds,
      this.#preferred,
    );
    if (!result) {
      this.#reject(segment, `Not a valid ${LABELS[segment].toLowerCase()}`);
      return false;
    }
    this.#apply(result, motion, segment);
    return true;
  }

  /** Resolve a pending buffer (Tab, Enter, blur, separator, arrows). Invalid partials revert (spec §12). */
  #commitBuffer(segment) {
    if (!this.#buffer) return "empty";
    const value = core.resolveBuffer(
      this.#buffer,
      segment,
      this.#candidates(segment),
    );
    this.#buffer = "";
    if (value == null) {
      this.#renderSegment(segment);
      this.#syncScroller("jump");
      this.#announce(`Incomplete ${LABELS[segment].toLowerCase()} discarded`);
      return "discarded";
    }
    this.#commitValue(segment, value, "jump");
    return "committed";
  }

  #step(delta) {
    const segment = this.#active;
    if (!segment || !this.#editable) return;
    this.#commitBuffer(segment);
    this.#swallowSeparator = false;
    const wasEmpty = this.#parts[segment] == null;
    const result = core.stepSegment(
      this.#parts,
      segment,
      delta,
      this.#bounds,
      todayParts(),
      this.#preferred,
    );
    this.#apply(result, wasEmpty ? "jump" : "step", segment);
    this.#openScroller();
  }

  #setBoundary(which) {
    const segment = this.#active;
    if (!segment || !this.#editable) return;
    this.#buffer = "";
    const value = core.segmentBoundary(
      this.#parts,
      segment,
      this.#bounds,
      which,
    );
    this.#commitValue(segment, value, "jump");
  }

  #selectFromScroller(value) {
    const segment = this.#active;
    if (!segment || !this.#editable) return;
    this.#buffer = "";
    this.#swallowSeparator = false;
    this.#commitValue(segment, value, "jump");
  }

  #moveFocus(direction) {
    const i = this.#order.indexOf(this.#active);
    const next = this.#order[i + direction];
    if (next) this.#inputs[next].focus();
    return Boolean(next);
  }

  // ---- keyboard ----------------------------------------------------------------------

  #onKeyDown(e, segment) {
    if (e.ctrlKey || e.metaKey || e.altKey || e.isComposing) return;
    const handled = () => e.preventDefault();
    switch (e.key) {
      case "ArrowUp":
        handled();
        this.#step(1);
        return;
      case "ArrowDown":
        handled();
        this.#step(-1);
        return;
      case "ArrowLeft":
        handled();
        this.#swallowSeparator = false;
        this.#moveFocus(-1);
        return;
      case "ArrowRight":
        handled();
        this.#swallowSeparator = false;
        this.#moveFocus(1);
        return;
      case "Home":
        handled();
        this.#setBoundary("first");
        return;
      case "End":
        handled();
        this.#setBoundary("last");
        return;
      case "Enter":
        this.#commitBuffer(segment);
        this.#emitChangeIfNeeded();
        return;
      case "Escape":
        if (this.#buffer) {
          handled();
          this.#buffer = "";
          this.#renderSegment(segment);
          this.#syncScroller("jump");
        } else if (this.#scroller.isOpen) {
          handled();
          this.#closeScroller();
        }
        return;
      case "Backspace":
      case "Delete":
        handled();
        this.#erase(segment);
        return;
      default:
    }
  }

  /**
   * All character entry arrives here (desktop keys, mobile keyboards, IME
   * commits) and is cancelled so the input only ever shows rendered state.
   */
  #onBeforeInput(e, segment) {
    e.stopPropagation();
    if ((e.inputType ?? "").includes("Composition")) return; // not cancelable; see compositionend
    e.preventDefault();
    if (!this.#editable) return;
    const type = e.inputType ?? "";
    if (type.startsWith("delete")) {
      this.#erase(segment);
    } else if (type === "insertFromPaste" || type === "insertFromDrop") {
      this.#insertText(e.dataTransfer?.getData("text/plain") ?? e.data ?? "");
    } else if (type.startsWith("insert") && e.data) {
      this.#typeText(e.data);
    }
  }

  /** Fallback for platforms where beforeinput is not cancelable (some Android IMEs). */
  #onNativeInput(e, segment) {
    e.stopPropagation();
    // Composition text is handled once, on compositionend.
    if (
      this.#composing ||
      e.isComposing ||
      (e.inputType ?? "").includes("Composition")
    )
      return;
    if (this.#editable) {
      const type = e.inputType ?? "";
      if (type.startsWith("delete")) this.#erase(segment);
      else if (e.data) this.#typeText(e.data);
    }
    this.#renderSegment(segment, true);
  }

  #onPaste(e, segment) {
    e.preventDefault();
    e.stopPropagation();
    if (!this.#editable) return;
    this.#insertText(e.clipboardData?.getData("text/plain") ?? "");
    void segment;
  }

  /** A pasted full date replaces the whole value; digits are typed; anything else is rejected. */
  #insertText(text) {
    const parsed = core.parseLooseDate(text, this.order);
    if (parsed) {
      if (!core.isWithinBounds(parsed, this.#bounds)) {
        this.#reject(this.#active, "Pasted date is outside the allowed range");
        return;
      }
      const before = this.value;
      this.#invalidValue = null;
      this.#parts = parsed;
      this.#buffer = "";
      this.#seedPreferred(); // a pasted date is an explicit choice of every segment
      this.#renderAll();
      this.#updateFormValue();
      this.#syncScroller("jump");
      this.#userChanged(before);
      return;
    }
    this.#typeText(text.trim());
  }

  #typeText(text) {
    for (const ch of text) this.#typeChar(ch);
  }

  #typeChar(ch) {
    const segment = this.#active;
    if (!segment) return;
    if (/^\d$/.test(ch)) {
      this.#swallowSeparator = false;
      const result = core.processDigit(
        this.#buffer,
        ch,
        segment,
        this.#candidates(segment),
      );
      if (result.status === "reject") {
        const attempted = this.#buffer + ch;
        this.#reject(
          segment,
          `${attempted} is not a valid ${LABELS[segment].toLowerCase()}`,
        );
        return;
      }
      this.#openScroller();
      if (result.status === "pending") {
        this.#buffer = result.buffer;
        this.#renderSegment(segment);
        this.#syncScroller("jump");
        return;
      }
      this.#buffer = "";
      if (
        this.#commitValue(segment, result.value, "jump") &&
        this.autoAdvance &&
        this.#moveFocus(1)
      ) {
        this.#swallowSeparator = true;
      }
      return;
    }
    if (ch === this.separator || EXTRA_SEPARATORS.has(ch)) {
      // "09/" - the separator after an auto-advance belongs to the segment just left.
      if (this.#swallowSeparator && !this.#buffer) {
        this.#swallowSeparator = false;
        return;
      }
      this.#swallowSeparator = false;
      if (this.#commitBuffer(segment) === "discarded") return;
      this.#moveFocus(1);
      return;
    }
    this.#reject(
      segment,
      `Type digits for the ${LABELS[segment].toLowerCase()}`,
    );
  }

  #erase(segment) {
    if (!this.#editable) return;
    this.#swallowSeparator = false;
    if (this.#buffer) {
      this.#buffer = this.#buffer.slice(0, -1);
      this.#renderSegment(segment);
      this.#syncScroller("jump");
      return;
    }
    if (this.#parts[segment] == null) {
      // Backspace in an empty segment steps back, like deleting across a separator.
      this.#moveFocus(-1);
      return;
    }
    const before = this.value;
    this.#invalidValue = null;
    this.#parts = { ...this.#parts, [segment]: null };
    this.#renderAll();
    this.#updateFormValue();
    this.#syncScroller("none");
    this.#userChanged(before);
  }

  // ---- focus & scroller ----------------------------------------------------------------

  #onFocusIn(e) {
    const segment = e.target?.dataset?.segment;
    if (!segment) return;
    if (!this.#hasFocus) {
      this.#hasFocus = true;
      this.#valueAtFocus = this.value;
      this.#seedPreferred();
    }
    const changed = this.#active !== segment;
    this.#active = segment;
    this.#buffer = "";
    this.#renderSegment(segment);
    if (changed) this.#emit("focussegment", { segment });
    if (this.#scroller.isOpen) {
      // Same surface, repurposed for the newly focused segment (spec §7).
      if (changed) this.#scroller.markDatasetSwitch();
      this.#syncScroller("none");
      this.#scroller.reposition();
    } else {
      this.#openScroller();
    }
  }

  #onFocusOut(e) {
    const segment = e.target?.dataset?.segment;
    if (!segment) return;
    if (segment === this.#active) this.#commitBuffer(segment);
    this.#emit("blursegment", { segment });
    if (e.relatedTarget && this.#root.contains(e.relatedTarget)) return;
    if (!e.relatedTarget) {
      // Programmatic focus moves (el.focus()) and window switches report no relatedTarget.
      // Focusin for the next segment runs synchronously after this handler, so decide once
      // the move has settled: stay open only if focus is still on one of our segments.
      queueMicrotask(() => {
        if (
          this.#root.contains(document.activeElement) &&
          document.activeElement?.dataset?.segment &&
          document.hasFocus()
        )
          return;
        this.#leave();
      });
      return;
    }
    this.#leave();
  }

  /** Focus has left the whole control. */
  #leave() {
    if (!this.#hasFocus) return;
    this.#hasFocus = false;
    this.#active = null;
    this.#swallowSeparator = false;
    this.#preferred = {};
    this.#closeScroller();
    this.#emitChangeIfNeeded();
  }

  /** Clicks on padding or separators focus the nearest segment instead of the first. */
  #onFieldPointerDown(e) {
    if (e.target?.dataset?.segment || !this.#editable) return;
    e.preventDefault();
    let nearest = this.#order[0];
    let best = Infinity;
    for (const segment of this.#order) {
      const r = this.#inputs[segment].getBoundingClientRect();
      const d = Math.abs(e.clientX - (r.left + r.width / 2));
      if (d < best) {
        best = d;
        nearest = segment;
      }
    }
    this.#inputs[nearest].focus();
    this.#openScroller();
  }

  #openScroller() {
    if (
      !this.scroller ||
      !this.#active ||
      !this.#editable ||
      this.#scroller.isOpen
    )
      return;
    this.#syncScroller("none");
    this.#scroller.open();
    this.#emit("open", { segment: this.#active }, false);
  }

  #closeScroller() {
    if (!this.#scroller.isOpen) return;
    this.#scroller.close();
    this.#emit("close", {}, false);
  }

  /** Point the scroller at the active segment's values and centre the current one. */
  #syncScroller(motion) {
    const segment = this.#active;
    if (!segment || !this.scroller) return;
    const values = this.#candidates(segment);
    const display = segment === "month" ? this.monthDisplay : "numeric";
    const label =
      segment === "month" && display !== "numeric"
        ? (
            (names) => (v) =>
              names[v - 1]
          )(this.#monthLabels(display))
        : (v) => core.padSegment(segment, v);
    const itemWidth =
      segment === "year"
        ? ITEM_WIDTH.year
        : segment === "month" && display !== "numeric"
          ? ITEM_WIDTH[display]
          : ITEM_WIDTH[segment];
    const wraps = core.segmentWraps(segment, values.length);
    const switched = this.#scroller.setDataset(
      `${segment}|${display}`,
      values,
      label,
      wraps,
      itemWidth,
    );
    const current = this.#parts[segment];
    const committed =
      current ??
      core.suggestionFor(segment, this.#parts, this.#bounds, todayParts());
    // A pending buffer moves the strip to a preview; #parts is never touched here.
    const preview = this.#buffer
      ? core.previewFor(this.#buffer, segment, values, committed)
      : null;
    this.#scroller.setPreview(preview != null);
    this.#scroller.setEmpty(preview == null && current == null);
    this.#scroller.sync(preview ?? committed, switched ? "none" : motion);
  }

  // ---- events ----------------------------------------------------------------------------

  #emitChangeIfNeeded() {
    if (this.value === this.#valueAtFocus) return;
    this.#valueAtFocus = this.value;
    this.#emit("change");
  }

  #emit(type, detail, bubbles = true) {
    const init = { bubbles, composed: bubbles };
    const target = type === "input" || type === "change" ? this.#bridge : this;
    target.dispatchEvent(
      detail === undefined
        ? new Event(type, init)
        : new CustomEvent(type, { ...init, detail }),
    );
  }
}

if (!customElements.get("sn-segmented-date"))
  customElements.define("sn-segmented-date", SegmentedDatePicker);
