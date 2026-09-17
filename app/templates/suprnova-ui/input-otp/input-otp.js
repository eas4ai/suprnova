// sn-input-otp - per-character presentation for a one-time code (FORM-006).
// Light DOM: the input and the cells are the server's markup. The native
// input carries the whole code and submits it; this element only mirrors
// the typed characters into the aria-hidden cells and marks the next cell
// as active, so the control works before this file loads and after it is
// blocked. It carries no form value of its own.
if (!customElements.get("sn-input-otp")) {
  customElements.define(
    "sn-input-otp",
    class extends HTMLElement {
      // One connection's listeners and observer (UI-020): a disconnect
      // aborts them, and a morph that moves the element keeps them.
      #connection = null;

      connectedCallback() {
        this.#connection?.abort();
        this.#connection = null;
        const input = this.querySelector("input");
        const cells = Array.from(this.querySelectorAll(".sn-otp-cell"));
        if (!input || cells.length === 0) return;
        const connection = new AbortController();
        this.#connection = connection;
        const { signal } = connection;
        const mirror = () => {
          const value = input.value;
          cells.forEach((cell, index) => {
            const text = value.charAt(index);
            if (cell.textContent !== text) cell.textContent = text;
            const active = index === Math.min(value.length, cells.length - 1) && document.activeElement === input;
            if (active !== cell.hasAttribute("data-sn-active")) cell.toggleAttribute("data-sn-active", active);
          });
        };
        input.addEventListener("input", mirror, { signal });
        input.addEventListener("focus", mirror, { signal });
        input.addEventListener("blur", mirror, { signal });
        // A morph re-renders the cells from the server's empty markup and
        // resets the input, whose value the runtime restores in a later
        // phase of the same render without an input event. Mirror now,
        // writing only what differs so the observer settles, and once more
        // on the next frame, after that restoration.
        let frame = 0;
        const remirror = () => {
          mirror();
          if (frame !== 0) return;
          frame = requestAnimationFrame(() => {
            frame = 0;
            if (!signal.aborted) mirror();
          });
        };
        const observer = new MutationObserver(remirror);
        observer.observe(this.querySelector(".sn-otp-cells"), { childList: true, characterData: true, subtree: true });
        signal.addEventListener("abort", () => observer.disconnect(), { once: true });
        if (!this.hasAttribute("data-sn-upgraded")) this.setAttribute("data-sn-upgraded", "");
        mirror();
      }

      // A morph that moves the element keeps its connection.
      connectedMoveCallback() {}

      disconnectedCallback() {
        this.#connection?.abort();
        this.#connection = null;
      }
    },
  );
}
