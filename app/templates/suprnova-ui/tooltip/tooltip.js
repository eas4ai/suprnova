// sn-tooltip - dismisses the bubble the stylesheet shows (OVL-008). Light
// DOM: the trigger and the bubble are the server's markup, and the bubble
// still shows on hover and on focus with this script absent (OVL-002). The
// element only marks itself dismissed, which the stylesheet reads, and drops
// that mark when the pointer or focus leaves, so the next hover or focus
// shows the bubble again.
if (!customElements.get("sn-tooltip")) {
  customElements.define(
    "sn-tooltip",
    class extends HTMLElement {
      // One connection's listeners (UI-020): a disconnect aborts them, and a
      // morph that moves the element keeps them.
      #connection = null;

      connectedCallback() {
        this.#connection?.abort();
        const connection = new AbortController();
        this.#connection = connection;
        const signal = connection.signal;
        const shown = () => {
          // The bubble is shown where the pointer rests on this tooltip or
          // focus is inside it, which is where a dismissal has something to
          // dismiss.
          try {
            if (this.matches(":hover")) return true;
          } catch {
            // An engine that refuses the selector leaves focus as the test.
          }
          const active = this.ownerDocument.activeElement;
          return active !== null && this.contains(active);
        };
        // Escape reaches the document, not this element, while the pointer
        // rests on the trigger and focus is elsewhere.
        this.ownerDocument.addEventListener(
          "keydown",
          (event) => {
            if (event.key !== "Escape" || event.defaultPrevented || !shown()) return;
            this.setAttribute("data-sn-dismissed", "");
          },
          { signal },
        );
        this.addEventListener("pointerleave", () => this.#restore(), { signal });
        this.addEventListener(
          "focusout",
          (event) => {
            const next = event.relatedTarget;
            if (next instanceof Node && this.contains(next)) return;
            this.#restore();
          },
          { signal },
        );
      }

      // A morph that moves the element keeps its connection.
      connectedMoveCallback() {}

      disconnectedCallback() {
        this.#connection?.abort();
        this.#connection = null;
      }

      #restore() {
        this.removeAttribute("data-sn-dismissed");
      }
    },
  );
}
