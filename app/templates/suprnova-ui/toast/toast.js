// suprnova.toast - the region's timing, queue bound and dismissal
// (FDB-004). The live region announces a toast when the server inserts it;
// this element only decides when a toast leaves: after its duration, paused
// while the pointer or focus is inside it, or on its dismiss button. It
// never moves focus and never talks to the server. Light DOM: the toasts
// are the server's markup; hidden is the one attribute it writes, on a
// keyed, preserved root the browser owns across a morph.
if (!customElements.get("sn-toast-region")) {
  customElements.define(
    "sn-toast-region",
    class extends HTMLElement {
      #timers = new Map();
      #observer = null;

      connectedCallback() {
        this.addEventListener("click", this.#onClick);
        this.addEventListener("pointerenter", this.#pause, true);
        this.addEventListener("pointerleave", this.#resume, true);
        this.addEventListener("focusin", this.#pause);
        this.addEventListener("focusout", this.#resume);
        this.#observer = new MutationObserver(() => this.#schedule());
        this.#observer.observe(this, { childList: true, subtree: true });
        this.#schedule();
      }

      disconnectedCallback() {
        this.removeEventListener("click", this.#onClick);
        this.removeEventListener("pointerenter", this.#pause, true);
        this.removeEventListener("pointerleave", this.#resume, true);
        this.removeEventListener("focusin", this.#pause);
        this.removeEventListener("focusout", this.#resume);
        this.#observer?.disconnect();
        this.#observer = null;
        for (const timer of this.#timers.values()) clearTimeout(timer.handle);
        this.#timers.clear();
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
        timer.handle = setTimeout(() => this.#dismiss(toast), duration);
        this.#timers.set(toast, timer);
      }

      #dismiss(toast) {
        const timer = this.#timers.get(toast);
        if (timer) clearTimeout(timer.handle);
        this.#timers.delete(toast);
        toast.setAttribute("data-sn-dismissed", "");
        toast.hidden = true;
        this.#schedule();
      }

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
