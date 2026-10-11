// sn-sheet - hosts one native dialog used as a sheet. The dialog element owns
// the open state, focus containment and Escape (OVL-001); this element only
// answers the invoker's click with showModal(), closes on a close button or
// a backdrop click where the browser has no closedby, and returns focus on
// close to the invoker, or to itself when the invoker left the document
// (OVL-005). Light DOM: the dialog and its content are the server's markup.
if (!customElements.get("sn-sheet")) {
  customElements.define(
    "sn-sheet",
    class extends HTMLElement {
      // One connection's listeners (UI-020): a disconnect aborts them, and a
      // morph that moves the element keeps them.
      #connection = null;

      connectedCallback() {
        this.#connection?.abort();
        this.#connection = null;
        const dialog = this.querySelector("dialog");
        if (!dialog) return;
        const connection = new AbortController();
        this.#connection = connection;
        const { signal } = connection;
        let invoker = null;
        document.addEventListener(
          "click",
          (event) => {
            const target = event.target instanceof Element ? event.target : null;
            const opener = target && target.closest(`[data-sn-sheet-open="${dialog.id}"]`);
            if (opener) {
              invoker = opener;
              if (!dialog.open) dialog.showModal();
              return;
            }
            const closer = target && target.closest(`[data-sn-sheet-close="${dialog.id}"]`);
            if (closer && dialog.open) dialog.close();
          },
          { signal },
        );
        if (!("closedBy" in dialog)) {
          dialog.addEventListener(
            "click",
            (event) => {
              if (event.target === dialog) dialog.close();
            },
            { signal },
          );
        }
        dialog.addEventListener(
          "close",
          () => {
            const home = invoker && invoker.isConnected ? invoker : this;
            invoker = null;
            home.focus();
          },
          { signal },
        );
      }

      // A morph that moves the element keeps its connection and state.
      connectedMoveCallback() {}

      disconnectedCallback() {
        this.#connection?.abort();
        this.#connection = null;
      }
    },
  );
}
