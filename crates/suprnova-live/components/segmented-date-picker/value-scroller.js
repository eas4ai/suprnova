/**
 * Suprnova value scroller - the contextual manipulation surface (spec §15-§27).
 *
 * Owns no date state. The host tells it which values exist and which one is
 * current (`setDataset`, `sync`); it reports user intent back through
 * `onSelect(value)` and `onStep(delta)`. Position `p` is a continuous index
 * into the value list and may run past either end when the list wraps.
 */

const MOTION = Object.freeze({
  // Spec §21: adjacent 140-190 ms travel, 2-4 px overshoot, 80-120 ms settle.
  step: { main: 160, settle: 100 },
  // Typed destination: 180-250 ms total, entered from at most two values away.
  jump: { main: 150, settle: 90 },
  tap: { main: 180, settle: 100 },
  snap: { main: 140, settle: 90 },
  fling: { main: 0, settle: 110 },
});
const MAX_OVERSHOOT_PX = 3;
const JUMP_NEIGHBORHOOD = 2;
const DRAG_SLOP_PX = 6;
const MOMENTUM_TIME_CONSTANT_MS = 325;
const MIN_FLING_VELOCITY = 0.0005; // items per ms
const WHEEL_IDLE_MS = 140;
const WHEEL_THRESHOLD_PX = 8;
const WHEEL_NOTCH_PX = 50;
const REPEAT_DELAY_MS = 380;
const REPEAT_INTERVAL_MS = 85;
const MIN_WIDTH_PX = 272;
const VIEWPORT_MARGIN_PX = 8;
const GAP_PX = 10;

// Spec §23 emphasis by distance from centre: [distance, scale, opacity].
const EMPHASIS = [
  [0, 1, 1],
  [1, 0.96, 0.7],
  [2, 0.92, 0.4],
  [3, 0.9, 0.15],
];

const easeOutCubic = (t) => 1 - (1 - t) ** 3;
const easeInOutSine = (t) => -(Math.cos(Math.PI * t) - 1) / 2;
const mod = (a, n) => ((a % n) + n) % n;
const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));

function emphasisAt(distance) {
  if (distance >= EMPHASIS[EMPHASIS.length - 1][0])
    return EMPHASIS[EMPHASIS.length - 1].slice(1);
  const i = Math.floor(distance);
  const f = distance - i;
  const [, s0, o0] = EMPHASIS[i];
  const [, s1, o1] = EMPHASIS[i + 1];
  return [s0 + (s1 - s0) * f, o0 + (o1 - o0) * f];
}

const CHEVRON = (d) =>
  `<svg width="18" height="18" viewBox="0 0 18 18" aria-hidden="true"><path d="${d}" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>`;

export class ValueScroller {
  /**
   * @param {HTMLElement} mount  light-DOM editor to append to
   * @param {object} hooks
   *   onSelect(value)     user settled on a value by drag, wheel or tap
   *   onStep(delta)       user pressed previous/next
   *   getAnchor()         { field: DOMRect, segment: DOMRect } for positioning
   *   motionEnabled()     false when motion is off or reduced motion is requested
   */
  constructor(mount, hooks) {
    this.hooks = hooks;
    this.values = [];
    this.label = String;
    this.wraps = false;
    this.itemWidth = 48;
    this.p = 0;
    this.target = 0;
    this.anim = null;
    this.raf = 0;
    this.drag = null;
    this.wheel = null;
    this.repeat = null;
    this.isOpen = false;
    this.pool = [];
    this.viewportWidth = 0;
    this.datasetKey = "";

    const el = document.createElement("div");
    el.className = "sn-segmented-scroller";
    el.setAttribute("part", "scroller");
    // The segments are the accessible control; this surface is a pointer
    // convenience and must not present a second copy of the value (spec §35).
    el.setAttribute("aria-hidden", "true");
    if (typeof el.showPopover === "function")
      el.setAttribute("popover", "manual");
    el.innerHTML = `
      <span class="sn-segmented-notch"></span>
      <button class="sn-segmented-nav sn-segmented-prev" part="scroller-button" type="button" tabindex="-1">${CHEVRON("M11 4 6 9l5 5")}</button>
      <div class="sn-segmented-viewport">
        <div class="sn-segmented-selection" part="scroller-selection"></div>
        <div class="sn-segmented-track"></div>
      </div>
      <button class="sn-segmented-nav sn-segmented-next" part="scroller-button" type="button" tabindex="-1">${CHEVRON("m7 4 5 5-5 5")}</button>`;
    el.hidden = true;
    mount.append(el);
    this.el = el;
    this.viewport = el.querySelector(".sn-segmented-viewport");
    this.track = el.querySelector(".sn-segmented-track");
    this.prevButton = el.querySelector(".sn-segmented-prev");
    this.nextButton = el.querySelector(".sn-segmented-next");

    this.tick = this.tick.bind(this);
    this.onPointerDown = this.onPointerDown.bind(this);
    this.onPointerMove = this.onPointerMove.bind(this);
    this.onPointerUp = this.onPointerUp.bind(this);
    this.onPointerCancel = this.onPointerCancel.bind(this);
    this.onWheel = this.onWheel.bind(this);
    this.stopRepeat = this.stopRepeat.bind(this);
    this.scheduleReposition = this.scheduleReposition.bind(this);

    // Keep focus on the segment: nothing in the scroller takes focus.
    el.addEventListener("pointerdown", (e) => e.preventDefault());
    this.viewport.addEventListener("pointerdown", this.onPointerDown);
    this.viewport.addEventListener("pointermove", this.onPointerMove);
    this.viewport.addEventListener("pointerup", this.onPointerUp);
    this.viewport.addEventListener("pointercancel", this.onPointerCancel);
    // Touch pointers are implicitly captured by the item under the finger;
    // moving capture to the viewport fires lostpointercapture on that item,
    // which must not cancel the drag. Only the viewport losing it matters.
    this.viewport.addEventListener("lostpointercapture", (e) => {
      if (e.target === this.viewport) this.onPointerCancel(e);
    });
    el.addEventListener("wheel", this.onWheel, { passive: false });
    for (const [button, delta] of [
      [this.prevButton, -1],
      [this.nextButton, 1],
    ]) {
      button.addEventListener("pointerdown", (e) => {
        if (e.button !== 0 || button.disabled) return;
        this.startRepeat(delta);
      });
      button.addEventListener("pointerup", this.stopRepeat);
      button.addEventListener("pointerleave", this.stopRepeat);
      button.addEventListener("pointercancel", this.stopRepeat);
    }
  }

  // ---- data ---------------------------------------------------------------

  /** A pending edit is a preview, not a committed selection. */
  setPreview(preview) {
    this.el.toggleAttribute("data-sn-preview", preview);
  }

  /** Replace the list. Returns true when the list actually changed. */
  setDataset(key, values, label, wraps, itemWidth) {
    const signature = `${key}|${wraps}|${itemWidth}|${values.length}|${values[0]}|${values[values.length - 1]}`;
    if (signature === this.datasetKey && this.sameValues(values)) return false;
    this.datasetKey = signature;
    this.values = values;
    this.label = label;
    this.wraps = wraps;
    this.itemWidth = itemWidth;
    this.el.style.setProperty("--sn-segmented-item-width", `${itemWidth}px`);
    for (const item of this.pool) item.dataset.label = "";
    this.cancelMotion();
    // Relabel now: a following sync() may be a no-op when the index is unchanged.
    this.render();
    return true;
  }

  sameValues(values) {
    if (values.length !== this.values.length) return false;
    for (let i = 0; i < values.length; i++)
      if (values[i] !== this.values[i]) return false;
    return true;
  }

  valueAt(index) {
    const n = this.values.length;
    if (n === 0) return null;
    if (this.wraps) return this.values[mod(index, n)];
    return index >= 0 && index < n ? this.values[index] : null;
  }

  indexOf(value) {
    const i = this.values.indexOf(value);
    if (i >= 0) return i;
    let best = 0;
    for (let k = 1; k < this.values.length; k++) {
      if (
        Math.abs(this.values[k] - value) < Math.abs(this.values[best] - value)
      )
        best = k;
    }
    return best;
  }

  /** Mark the centre as a suggestion rather than a selection (empty segment). */
  setEmpty(empty) {
    this.el.toggleAttribute("data-sn-empty", empty);
  }

  /**
   * Bring `value` to the centre. motion: 'none' | 'step' | 'jump'.
   * Idempotent: re-syncing to the value already targeted does nothing, so a
   * selection the scroller itself reported does not restart its animation.
   */
  sync(value, motion) {
    if (this.values.length === 0) return;
    const base = this.indexOf(value);
    let target = base;
    if (this.wraps) {
      const n = this.values.length;
      const reference = Math.round(this.target);
      let delta = base - mod(reference, n);
      if (delta > n / 2) delta -= n;
      if (delta < -n / 2) delta += n;
      target = reference + delta;
    }
    if (target === this.target && (this.anim || this.p === target)) return;
    this.animateTo(target, motion);
  }

  /** Brief fade when the list is repurposed for another segment (spec §41). */
  markDatasetSwitch() {
    if (!this.hooks.motionEnabled()) return;
    this.track.removeAttribute("data-sn-switching");
    void this.track.offsetWidth;
    this.track.setAttribute("data-sn-switching", "");
  }

  // ---- motion -------------------------------------------------------------

  cancelMotion() {
    this.anim = null;
    if (this.raf) cancelAnimationFrame(this.raf);
    this.raf = 0;
  }

  animateTo(target, kind, opts = {}) {
    this.target = target;
    if (kind === "none" || !this.hooks.motionEnabled()) {
      this.cancelMotion();
      this.p = target;
      this.render();
      return;
    }
    let from = this.p;
    const distance = target - from;
    if (distance === 0) {
      this.cancelMotion();
      this.render();
      return;
    }
    // Spec §21: large typed jumps enter the destination's neighbourhood
    // instead of animating through every intermediate value.
    if (kind === "jump" && Math.abs(distance) > JUMP_NEIGHBORHOOD) {
      from = target - Math.sign(distance) * JUMP_NEIGHBORHOOD;
      this.p = from;
    }
    const timing = MOTION[kind] ?? MOTION.step;
    const main = opts.main ?? timing.main;
    const overshootPx = Math.min(
      MAX_OVERSHOOT_PX,
      Math.abs(target - from) * this.itemWidth * 0.08,
    );
    const peak =
      target + (Math.sign(target - from) * overshootPx) / this.itemWidth;
    this.anim = {
      from,
      peak,
      to: target,
      start: performance.now(),
      main,
      settle: timing.settle,
    };
    if (!this.raf) this.raf = requestAnimationFrame(this.tick);
  }

  tick(now) {
    this.raf = 0;
    const a = this.anim;
    if (!a) return;
    const t = now - a.start;
    if (t < a.main) {
      this.p = a.from + (a.peak - a.from) * easeOutCubic(t / a.main);
    } else if (t < a.main + a.settle) {
      this.p =
        a.peak + (a.to - a.peak) * easeInOutSine((t - a.main) / a.settle);
    } else {
      this.p = a.to;
      this.anim = null;
      this.normalizePosition();
    }
    this.render();
    if (this.anim) this.raf = requestAnimationFrame(this.tick);
  }

  /** Keep wrapped positions small; visually identical because rendering is modular. */
  normalizePosition() {
    if (!this.wraps || this.values.length === 0) return;
    const n = this.values.length;
    const shift = Math.floor(this.p / n) * n;
    if (shift !== 0 && Number.isInteger(this.target)) {
      this.p -= shift;
      this.target -= shift;
    }
  }

  // ---- rendering ----------------------------------------------------------

  measure() {
    this.viewportWidth = this.viewport.clientWidth;
  }

  ensurePool() {
    const needed = Math.ceil(this.viewportWidth / this.itemWidth) + 4;
    while (this.pool.length < needed) {
      const item = document.createElement("div");
      item.className = "sn-segmented-item";
      item.setAttribute("part", "scroller-item");
      this.track.append(item);
      this.pool.push(item);
    }
    for (let k = 0; k < this.pool.length; k++)
      this.pool[k].hidden = k >= needed;
    return needed;
  }

  render() {
    if (!this.isOpen || this.values.length === 0) return;
    if (!this.viewportWidth) this.measure();
    const count = this.ensurePool();
    const half = this.viewportWidth / 2;
    const w = this.itemWidth;
    const first = Math.floor(this.p) - Math.floor(count / 2);
    for (let k = 0; k < count; k++) {
      const item = this.pool[k];
      const index = first + k;
      const value = this.valueAt(index);
      if (value == null) {
        item.style.visibility = "hidden";
        continue;
      }
      const text = this.label(value);
      if (item.dataset.label !== text) {
        item.textContent = text;
        item.dataset.label = text;
      }
      item.dataset.index = String(index);
      const distance = Math.abs(index - this.p);
      const [scale, opacity] = emphasisAt(distance);
      item.style.visibility = "";
      item.style.transform = `translate3d(${half + (index - this.p) * w - w / 2}px,0,0) scale(${scale})`;
      item.style.opacity = String(opacity);
      item.setAttribute("aria-current", String(distance < 0.5));
    }
    const n = this.values.length;
    const at = Math.round(this.target);
    this.prevButton.disabled = !this.wraps && at <= 0;
    this.nextButton.disabled = !this.wraps && at >= n - 1;
  }

  // ---- open / close / position ---------------------------------------------

  open() {
    if (this.isOpen) return;
    this.isOpen = true;
    this.el.hidden = false;
    if (this.el.showPopover) {
      if (!this.el.matches(":popover-open")) this.el.showPopover();
    }
    if (this.hooks.motionEnabled()) {
      this.el.removeAttribute("data-sn-entering");
      void this.el.offsetWidth;
      this.el.setAttribute("data-sn-entering", "");
    }
    this.reposition();
    this.render();
    window.addEventListener("scroll", this.scheduleReposition, {
      capture: true,
      passive: true,
    });
    window.addEventListener("resize", this.scheduleReposition, {
      passive: true,
    });
    window.visualViewport?.addEventListener("resize", this.scheduleReposition, {
      passive: true,
    });
    window.visualViewport?.addEventListener("scroll", this.scheduleReposition, {
      passive: true,
    });
  }

  close() {
    if (!this.isOpen) return;
    this.isOpen = false;
    this.stopRepeat();
    cancelAnimationFrame(this.repositionFrame);
    this.repositionFrame = 0;
    this.el.removeAttribute("data-sn-dragging");
    this.endWheel(false);
    this.drag = null;
    this.cancelMotion();
    this.p = this.target;
    this.el.hidden = true;
    this.el.removeAttribute("data-sn-entering");
    if (this.el.hidePopover) {
      if (this.el.matches(":popover-open")) this.el.hidePopover();
    }
    window.removeEventListener("scroll", this.scheduleReposition, {
      capture: true,
    });
    window.removeEventListener("resize", this.scheduleReposition);
    window.visualViewport?.removeEventListener(
      "resize",
      this.scheduleReposition,
    );
    window.visualViewport?.removeEventListener(
      "scroll",
      this.scheduleReposition,
    );
  }

  scheduleReposition() {
    if (this.repositionFrame) return;
    this.repositionFrame = requestAnimationFrame(() => {
      this.repositionFrame = 0;
      if (this.isOpen) this.reposition();
    });
  }

  /** Spec §26: below and centred on the field, 100-140% of its width, flipped above when the keyboard or viewport edge leaves no room. */
  reposition() {
    const { field, segment } = this.hooks.getAnchor();
    const vv = window.visualViewport;
    const viewLeft = vv ? vv.offsetLeft : 0;
    const viewTop = vv ? vv.offsetTop : 0;
    const viewWidth = vv ? vv.width : window.innerWidth;
    const viewHeight = vv ? vv.height : window.innerHeight;

    // 120% of the field, never below the minimum that shows three values plus
    // two 44px buttons, never wider than the visible viewport.
    const width = Math.min(
      Math.max(field.width * 1.2, MIN_WIDTH_PX),
      viewWidth - VIEWPORT_MARGIN_PX * 2,
    );
    const left = clamp(
      field.left + field.width / 2 - width / 2,
      viewLeft + VIEWPORT_MARGIN_PX,
      viewLeft + viewWidth - VIEWPORT_MARGIN_PX - width,
    );
    this.el.style.width = `${width}px`;
    const height = this.el.offsetHeight || 60;
    const below = field.bottom + GAP_PX;
    const above = field.top - GAP_PX - height;
    const fitsBelow =
      below + height <= viewTop + viewHeight - VIEWPORT_MARGIN_PX;
    const fitsAbove = above >= viewTop + VIEWPORT_MARGIN_PX;
    const side = !fitsBelow && fitsAbove ? "top" : "bottom";
    this.el.dataset.side = side;
    this.el.style.left = `${left}px`;
    this.el.style.top = `${side === "top" ? above : below}px`;
    const notchX = clamp(
      segment.left + segment.width / 2 - left,
      18,
      width - 18,
    );
    this.el.style.setProperty("--sn-segmented-notch-x", `${notchX}px`);

    const previousWidth = this.viewportWidth;
    this.measure();
    if (this.viewportWidth !== previousWidth) this.render();
  }

  // ---- pointer drag (spec §22) ----------------------------------------------

  onPointerDown(e) {
    if (e.button !== 0 || this.values.length === 0) return;
    this.drag = {
      id: e.pointerId,
      startX: e.clientX,
      startY: e.clientY,
      startP: this.p,
      active: false,
      samples: [{ t: e.timeStamp, x: e.clientX }],
      index: e.target.closest?.(".sn-segmented-item")?.dataset.index ?? null,
    };
  }

  onPointerMove(e) {
    const d = this.drag;
    if (!d || e.pointerId !== d.id) return;
    const dx = e.clientX - d.startX;
    const dy = e.clientY - d.startY;
    if (!d.active) {
      if (Math.hypot(dx, dy) < DRAG_SLOP_PX) return;
      // Spec §38: only a clearly horizontal gesture becomes a drag.
      if (Math.abs(dy) >= Math.abs(dx)) {
        this.drag = null;
        return;
      }
      d.active = true;
      d.startX = e.clientX;
      d.startP = this.p;
      this.cancelMotion();
      this.viewport.setPointerCapture(e.pointerId);
      this.el.setAttribute("data-sn-dragging", "");
    }
    this.p = this.resist(d.startP - (e.clientX - d.startX) / this.itemWidth);
    d.samples.push({ t: e.timeStamp, x: e.clientX });
    while (d.samples.length > 2 && e.timeStamp - d.samples[0].t > 100)
      d.samples.shift();
    this.render();
  }

  /** Rubber-band past the ends of a non-wrapping list (year bounds). */
  resist(p) {
    if (this.wraps) return p;
    const last = this.values.length - 1;
    const band = (x) => 0.6 * (1 - 1 / (1 + x));
    if (p < 0) return -band(-p);
    if (p > last) return last + band(p - last);
    return p;
  }

  onPointerUp(e) {
    const d = this.drag;
    if (!d || e.pointerId !== d.id) return;
    this.drag = null;
    this.el.removeAttribute("data-sn-dragging");
    if (!d.active) {
      if (d.index != null) this.select(Number(d.index), "tap");
      return;
    }
    // Velocity over the 100 ms before release. A finger that stopped before
    // lifting has no recent samples and therefore no momentum.
    const samples = d.samples.filter((s) => e.timeStamp - s.t <= 100);
    samples.push({ t: e.timeStamp, x: e.clientX });
    const first = samples[0];
    const dt = e.timeStamp - first.t;
    const velocity =
      samples.length > 1 && dt > 0
        ? -(e.clientX - first.x) / this.itemWidth / dt
        : 0;
    this.release(velocity);
  }

  onPointerCancel(e) {
    const d = this.drag;
    if (!d || (e.pointerId != null && e.pointerId !== d.id)) return;
    this.drag = null;
    this.el.removeAttribute("data-sn-dragging");
    if (d.active) this.release(0);
  }

  release(velocity) {
    const n = this.values.length;
    const limit = (i) => (this.wraps ? i : clamp(i, 0, n - 1));
    if (!this.hooks.motionEnabled()) {
      // Spec §36: no momentum, no spring - snap to the nearest value.
      this.select(limit(Math.round(this.p)), "none");
      return;
    }
    const projected = this.p + velocity * MOMENTUM_TIME_CONSTANT_MS;
    const target = limit(Math.round(projected));
    const distance = target - this.p;
    if (
      Math.abs(velocity) < MIN_FLING_VELOCITY ||
      Math.sign(distance) !== Math.sign(velocity) ||
      distance === 0
    ) {
      this.select(target, "snap");
      return;
    }
    // Start the ease-out at the finger's release velocity: v0 = 3·d/T.
    const main = clamp((3 * Math.abs(distance)) / Math.abs(velocity), 180, 700);
    this.select(target, "fling", { main });
  }

  /** Settle on an index and report it. The value is decided here, before the settle plays. */
  select(index, kind, opts) {
    this.animateTo(index, kind, opts);
    const value = this.valueAt(index);
    if (value != null) this.hooks.onSelect(value);
  }

  // ---- wheel / trackpad (spec §25) -------------------------------------------

  onWheel(e) {
    if (!this.isOpen || this.values.length === 0) return;
    const unit =
      e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? this.viewportWidth : 1;
    const dx = e.deltaX * unit;
    const dy = e.deltaY * unit;
    const horizontal = Math.abs(dx) > Math.abs(dy);
    const notch =
      !horizontal &&
      (e.deltaMode !== 0 || (Math.abs(dy) >= WHEEL_NOTCH_PX && dx === 0));
    if (notch) {
      // A mouse wheel detent is one step. Continuous vertical trackpad motion
      // is left to the page so the scroller never hijacks page scrolling.
      e.preventDefault();
      this.hooks.onStep(Math.sign(dy));
      return;
    }
    if (!horizontal) return;
    e.preventDefault();
    if (!this.wheel)
      this.wheel = { accumulated: 0, startP: this.p, active: false, timer: 0 };
    const w = this.wheel;
    w.accumulated += dx;
    if (!w.active) {
      if (Math.abs(w.accumulated) < WHEEL_THRESHOLD_PX) {
        this.armWheelIdle();
        return;
      }
      w.active = true;
      this.cancelMotion();
      w.startP = this.p;
    }
    this.p = this.resist(w.startP + w.accumulated / this.itemWidth);
    this.render();
    this.armWheelIdle();
  }

  armWheelIdle() {
    clearTimeout(this.wheel.timer);
    this.wheel.timer = setTimeout(() => this.endWheel(true), WHEEL_IDLE_MS);
  }

  endWheel(commit) {
    const w = this.wheel;
    if (!w) return;
    clearTimeout(w.timer);
    this.wheel = null;
    if (commit && w.active) this.release(0);
  }

  // ---- previous / next -------------------------------------------------------

  startRepeat(delta) {
    this.stopRepeat();
    this.hooks.onStep(delta);
    this.repeat = setTimeout(() => {
      this.repeat = setInterval(
        () => this.hooks.onStep(delta),
        REPEAT_INTERVAL_MS,
      );
    }, REPEAT_DELAY_MS);
  }

  stopRepeat() {
    if (this.repeat) {
      clearTimeout(this.repeat);
      clearInterval(this.repeat);
    }
    this.repeat = null;
  }

  destroy() {
    this.close();
    this.el.remove();
  }
}
