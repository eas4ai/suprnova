// suprnova.toast - the region's timing, queue bound and dismissal
// (FDB-004). The live region announces a toast when the server inserts it;
// this element only decides when a toast leaves: after its duration, held
// while the pointer is over any part of the region or focus is inside it
// (FDB-007), or on its dismiss button. It
// never moves focus and never talks to the server. Light DOM: the toasts
// are the server's markup; hidden is the one attribute it writes, on a
// keyed, preserved root the browser owns across a morph.
if (!customElements.get("sn-toast-region")) {
  customElements.define(
    "sn-toast-region",
    class extends HTMLElement {
      #timers = new Map();
      #observer = null;
      // Why the timers are held. pointerenter and pointerleave are heard on
      // the region itself, not captured from its descendants: moving between
      // a toast's text and its padding leaves a descendant but never the
      // region, so it must not resume the timers.
      #hovered = false;
      #focused = false;

      connectedCallback() {
        this.addEventListener("click", this.#onClick);
        this.addEventListener("pointerenter", this.#onPointerEnter);
        this.addEventListener("pointerleave", this.#onPointerLeave);
        this.addEventListener("focusin", this.#onFocusIn);
        this.addEventListener("focusout", this.#onFocusOut);
        this.#observer = new MutationObserver(() => this.#schedule());
        this.#observer.observe(this, { childList: true, subtree: true });
        this.#schedule();
      }

      disconnectedCallback() {
        this.removeEventListener("click", this.#onClick);
        this.removeEventListener("pointerenter", this.#onPointerEnter);
        this.removeEventListener("pointerleave", this.#onPointerLeave);
        this.removeEventListener("focusin", this.#onFocusIn);
        this.removeEventListener("focusout", this.#onFocusOut);
        this.#observer?.disconnect();
        this.#observer = null;
        for (const timer of this.#timers.values()) clearTimeout(timer.handle);
        this.#timers.clear();
        this.#hovered = false;
        this.#focused = false;
      }

      // A morph that moves the region keeps its timers and listeners.
      connectedMoveCallback() {}

      #held() {
        return this.#hovered || this.#focused;
      }

      #limit() {
        const limit = Number.parseInt(this.getAttribute("data-sn-limit") ?? "3", 10);
        return Number.isFinite(limit) && limit > 0 ? limit : 3;
      }

      #toasts() {
        return [...this.querySelectorAll(".sn-toast")];
      }

      #schedule() {
        let visible = 0;
        for (const toast of this.#toasts()) {
          if (toast.hasAttribute("data-sn-dismissed")) continue;
          if (visible < this.#limit()) {
            visible += 1;
            if (toast.hasAttribute("data-sn-queued")) {
              toast.removeAttribute("data-sn-queued");
              toast.hidden = false;
            }
            if (!this.#timers.has(toast)) this.#start(toast);
          } else if (!toast.hasAttribute("data-sn-queued")) {
            toast.setAttribute("data-sn-queued", "");
            toast.hidden = true;
          }
        }
      }

      #start(toast) {
        const duration = Number.parseInt(toast.getAttribute("data-sn-duration") ?? "6000", 10);
        if (!Number.isFinite(duration) || duration <= 0) return;
        const timer = { remaining: duration, startedAt: performance.now(), handle: 0 };
        // A toast that arrives while the timers are held starts held.
        if (!this.#held()) timer.handle = setTimeout(() => this.#dismiss(toast), duration);
        this.#timers.set(toast, timer);
      }

      #dismiss(toast) {
        const timer = this.#timers.get(toast);
        if (timer) clearTimeout(timer.handle);
        this.#timers.delete(toast);
        // Hiding the toast that holds focus takes focus with it, and not
        // every engine reports that as a focusout.
        const heldFocus = toast.contains(document.activeElement);
        toast.setAttribute("data-sn-dismissed", "");
        toast.hidden = true;
        if (heldFocus) this.#focused = false;
        this.#schedule();
        this.#resume();
      }

      #onPointerEnter = () => {
        this.#hovered = true;
        this.#pause();
      };

      #onPointerLeave = () => {
        this.#hovered = false;
        this.#resume();
      };

      #onFocusIn = () => {
        this.#focused = true;
        this.#pause();
      };

      #onFocusOut = (event) => {
        if (event.relatedTarget instanceof Node && this.contains(event.relatedTarget)) return;
        this.#focused = false;
        this.#resume();
      };

      #pause = () => {
        for (const [toast, timer] of this.#timers) {
          if (timer.handle === 0) continue;
          clearTimeout(timer.handle);
          timer.remaining -= performance.now() - timer.startedAt;
          timer.handle = 0;
          void toast;
        }
      };

      #resume = () => {
        if (this.#held()) return;
        for (const [toast, timer] of this.#timers) {
          if (timer.handle !== 0) continue;
          timer.startedAt = performance.now();
          timer.handle = setTimeout(() => this.#dismiss(toast), Math.max(timer.remaining, 0));
        }
      };

      #onClick = (event) => {
        const button = event.target instanceof Element ? event.target.closest("[data-sn-toast-dismiss]") : null;
        const toast = button?.closest(".sn-toast");
        if (toast && this.contains(toast)) this.#dismiss(toast);
      };
    },
  );
}
