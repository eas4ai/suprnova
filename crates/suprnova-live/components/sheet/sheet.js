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
      connectedCallback() {
        const dialog = this.querySelector("dialog");
        if (!dialog || this.hasAttribute("data-sn-ready")) return;
        this.setAttribute("data-sn-ready", "");
        let invoker = null;
        document.addEventListener("click", (event) => {
          const target = event.target instanceof Element ? event.target : null;
          const opener = target && target.closest(`[data-sn-sheet-open="${dialog.id}"]`);
          if (opener) {
            invoker = opener;
            if (!dialog.open) dialog.showModal();
            return;
          }
          const closer = target && target.closest(`[data-sn-sheet-close="${dialog.id}"]`);
          if (closer && dialog.open) dialog.close();
        });
        if (!("closedBy" in dialog)) {
          dialog.addEventListener("click", (event) => {
            if (event.target === dialog) dialog.close();
          });
        }
        dialog.addEventListener("close", () => {
          const home = invoker && invoker.isConnected ? invoker : this;
          invoker = null;
          home.focus();
        });
      }
    },
  );
}
