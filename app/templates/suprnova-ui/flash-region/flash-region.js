// suprnova.flash-region - the optional timeout of a flash region (FDB-004).
// A region rendered with a duration hides each message that long after it
// appears, held while the pointer is over the region or focus is inside it,
// so a reader who reaches for a message keeps it. The default region has no
// duration, and this element then does nothing: its messages stay until
// the next document. Light DOM: the messages are the server's markup; hidden is the
// one attribute this element writes, and a morph that shows a message again
// times it again.
if (!customElements.get("sn-flash-region")) {
  customElements.define(
    "sn-flash-region",
    class extends HTMLElement {
      #timers = new Map();
      #observer = null;
      // Why the timers are held. pointerenter and pointerleave are heard on
      // the region itself, not captured from its descendants: moving between
      // a message's text and its padding leaves a descendant but never the
      // region, so it must not resume the timers.
      #hovered = false;
      #focused = false;

      connectedCallback() {
        this.addEventListener("pointerenter", this.#onPointerEnter);
        this.addEventListener("pointerleave", this.#onPointerLeave);
        this.addEventListener("focusin", this.#onFocusIn);
        this.addEventListener("focusout", this.#onFocusOut);
        document.addEventListener("pointerover", this.#onDocumentPointerOver);
        this.#observer = new MutationObserver(() => this.#settle());
        this.#observer.observe(this, {
          childList: true,
          subtree: true,
          attributes: true,
          attributeFilter: ["hidden"],
        });
        this.#settle();
      }

      disconnectedCallback() {
        this.removeEventListener("pointerenter", this.#onPointerEnter);
        this.removeEventListener("pointerleave", this.#onPointerLeave);
        this.removeEventListener("focusin", this.#onFocusIn);
        this.removeEventListener("focusout", this.#onFocusOut);
        document.removeEventListener("pointerover", this.#onDocumentPointerOver);
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

      #duration() {
        const duration = Number.parseInt(this.getAttribute("data-sn-duration") ?? "0", 10);
        return Number.isFinite(duration) && duration > 0 ? duration : 0;
      }

      // After the region's content changes: forget the messages that left
      // it or were hidden, then time every shown message that has no timer.
      // A morph that removes a message would otherwise leave its timer
      // running for as long as the page lives; one that shows a hidden
      // message again, by rendering it without the attribute, times it
      // again.
      #settle() {
        for (const [message, timer] of this.#timers) {
          if (this.contains(message) && !message.hidden) continue;
          clearTimeout(timer.handle);
          this.#timers.delete(message);
        }
        // Removing the element that holds focus moves focus to the body, and
        // not every engine reports that as a focusout, which would leave the
        // remaining messages held for good.
        if (this.#focused && !this.contains(document.activeElement)) {
          this.#focused = false;
          this.#resume();
        }
        const duration = this.#duration();
        if (duration === 0) return;
        for (const message of this.querySelectorAll(".sn-flash")) {
          if (message.hidden || this.#timers.has(message)) continue;
          const timer = { remaining: duration, startedAt: performance.now(), handle: 0 };
          // A message that appears while the timers are held starts held.
          if (!this.#held()) timer.handle = setTimeout(() => this.#hide(message), duration);
          this.#timers.set(message, timer);
        }
      }

      #hide(message) {
        const timer = this.#timers.get(message);
        if (timer) clearTimeout(timer.handle);
        this.#timers.delete(message);
        // Hiding the message that holds focus takes focus with it, and not
        // every engine reports that as a focusout.
        const heldFocus = message.contains(document.activeElement);
        message.hidden = true;
        if (heldFocus) this.#focused = false;
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

      // The region's own pointerleave does not always arrive: an engine can
      // lose it when the message under the pointer is replaced, leaving every
      // timer held after the pointer has gone. The pointer entering any
      // element outside the region says the same thing.
      #onDocumentPointerOver = (event) => {
        if (!this.#hovered) return;
        if (event.target instanceof Node && this.contains(event.target)) return;
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
        for (const timer of this.#timers.values()) {
          if (timer.handle === 0) continue;
          clearTimeout(timer.handle);
          timer.remaining -= performance.now() - timer.startedAt;
          timer.handle = 0;
        }
      };

      #resume = () => {
        if (this.#held()) return;
        for (const [message, timer] of this.#timers) {
          if (timer.handle !== 0) continue;
          timer.startedAt = performance.now();
          timer.handle = setTimeout(() => this.#hide(message), Math.max(timer.remaining, 0));
        }
      };
    },
  );
}
