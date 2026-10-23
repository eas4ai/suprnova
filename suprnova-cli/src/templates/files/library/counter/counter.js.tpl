// {namespace}-counter - while focus is inside the counter, the plus key
// presses its button, so the count can be raised from the keyboard. Light
// DOM: the button is the server's markup and runs the increment action;
// this element only forwards the key.
if (!customElements.get("{namespace}-counter")) {
  customElements.define(
    "{namespace}-counter",
    class extends HTMLElement {
      // One connection's listener: a disconnect aborts it, and a morph that
      // moves the element keeps it.
      #connection = null;

      connectedCallback() {
        this.#connection?.abort();
        const connection = new AbortController();
        this.#connection = connection;
        this.addEventListener(
          "keydown",
          (event) => {
            if (event.key !== "+") return;
            // Read at each press: a morph may have replaced the button.
            const button = this.querySelector("button");
            if (!button) return;
            event.preventDefault();
            button.click();
          },
          { signal: connection.signal },
        );
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
