// sn-password-reveal - reveals or hides the password in the input it wraps.
// Light DOM: the input and the button are the server's markup; this element
// only toggles the input's type and mirrors the state on the button.
if (!customElements.get("sn-password-reveal")) {
  customElements.define(
    "sn-password-reveal",
    class extends HTMLElement {
      connectedCallback() {
        const button = this.querySelector("button[aria-controls]");
        const input = this.querySelector("input");
        if (!button || !input) return;
        button.addEventListener("click", () => {
          const reveal = input.type === "password";
          input.type = reveal ? "text" : "password";
          button.setAttribute("aria-pressed", String(reveal));
          button.textContent = reveal ? "Hide" : "Show";
        });
      }
    },
  );
}
