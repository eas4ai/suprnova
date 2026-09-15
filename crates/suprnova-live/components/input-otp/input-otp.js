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
      connectedCallback() {
        const input = this.querySelector("input");
        const cells = Array.from(this.querySelectorAll(".sn-otp-cell"));
        if (!input || cells.length === 0) return;
        const mirror = () => {
          const value = input.value;
          cells.forEach((cell, index) => {
            const text = value.charAt(index);
            if (cell.textContent !== text) cell.textContent = text;
            const active = index === Math.min(value.length, cells.length - 1) && document.activeElement === input;
            if (active !== cell.hasAttribute("data-sn-active")) cell.toggleAttribute("data-sn-active", active);
          });
        };
        input.addEventListener("input", mirror);
        input.addEventListener("focus", mirror);
        input.addEventListener("blur", mirror);
        // A morph re-renders the cells from the server's empty markup; mirror
        // again, writing only what differs so the observer settles.
        new MutationObserver(mirror).observe(this.querySelector(".sn-otp-cells"), { childList: true, characterData: true, subtree: true });
        this.setAttribute("data-sn-upgraded", "");
        mirror();
      }
    },
  );
}
