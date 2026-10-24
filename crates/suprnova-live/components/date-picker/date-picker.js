// sn-date-picker - composes the strip selection into the date input
// (FORM-007). Light DOM: the input, the details and the radio strips are
// the server's markup. The radios select natively; this element only joins
// a complete year, month and day into the input's value (which the model
// binding then sees) and mirrors a typed date back onto the strips. The
// input is the control and carries the value, so nothing here is
// form-associated and the control works with this file blocked.
if (!customElements.get("sn-date-picker")) {
  customElements.define(
    "sn-date-picker",
    class extends HTMLElement {
      // One connection's listeners and observer (UI-020): a disconnect
      // aborts them, and a morph that moves the element keeps them.
      #connection = null;

      connectedCallback() {
        this.#connection?.abort();
        this.#connection = null;
        const input = this.querySelector("input[type=date]");
        if (!input) return;
        const connection = new AbortController();
        this.#connection = connection;
        const { signal } = connection;
        const strip = (part) => this.querySelector(`[data-sn-part="${part}"]`);
        const checked = (part) => {
          const root = strip(part);
          const radio = root ? root.querySelector("input:checked") : null;
          return radio ? radio.value : "";
        };
        const check = (part, value) => {
          const root = strip(part);
          if (!root) return;
          for (const radio of root.querySelectorAll("input")) {
            const on = Number(radio.value) === Number(value);
            if (radio.checked === on) continue;
            radio.checked = on;
            if (on) radio.closest("label")?.scrollIntoView({ block: "nearest", inline: "center" });
          }
        };
        const compose = () => {
          const year = checked("year");
          const month = checked("month");
          const day = checked("day");
          if (!year || !month || !day) return;
          const next = `${year}-${month.padStart(2, "0")}-${day.padStart(2, "0")}`;
          if (input.value === next) return;
          input.value = next;
          input.dispatchEvent(new Event("input", { bubbles: true }));
          input.dispatchEvent(new Event("change", { bubbles: true }));
        };
        const mirror = () => {
          const [year, month, day] = input.value.split("-");
          if (!year || !month || !day) return;
          check("year", year);
          check("month", month);
          check("day", day);
        };
        this.addEventListener(
          "change",
          (event) => {
            if (event.target !== input && event.target instanceof HTMLInputElement && event.target.type === "radio") compose();
          },
          { signal },
        );
        input.addEventListener("change", mirror, { signal });
        // A morph re-renders the strips from the server's unchecked markup;
        // mirror the input back onto them, writing only what differs.
        const observer = new MutationObserver(mirror);
        observer.observe(this, { childList: true, subtree: true });
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
