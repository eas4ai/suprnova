// acme-disclosure: toggles its panel and reports the toggle to the
// application, same-origin, with the standard browser APIs a component
// script may use.
const ATTRIBUTE = "aria-expanded";

const mark = (element, name, value) => {
  if (element.getAttribute(name) !== value) element.setAttribute(name, value);
};

class Disclosure extends HTMLElement {
  #connection = null;
  #timer = 0;

  connectedCallback() {
    this.#connection?.abort();
    const connection = new AbortController();
    this.#connection = connection;
    const button = this.querySelector(".acme-disclosure-toggle");
    const panel = this.querySelector(".acme-disclosure-panel");
    if (!button || !panel) return;
    button.addEventListener("click", () => this.#toggle(button, panel), { signal: connection.signal });
    Array.from(this.querySelectorAll("a")).forEach((link) => {
      link.dataset.acmeSeen = "true";
    });
  }

  disconnectedCallback() {
    this.#connection?.abort();
    clearTimeout(this.#timer);
  }

  #toggle(button, panel) {
    const open = panel.hidden;
    panel.hidden = !open;
    mark(button, ATTRIBUTE, String(open));
    mark(this, "data-state", open ? "open" : "closed");
    const note = document.createElement("span");
    note.className = "acme-disclosure-note";
    note.textContent = `${open ? "Opened" : "Closed"} at ${new Date().toISOString()}`;
    panel.append(note);
    const help = document.createElement("a");
    help.href = "/help/disclosure";
    panel.append(help);
    this.#timer = setTimeout(() => note.remove(), 2000);
    fetch("/api/disclosure", { method: "POST", body: JSON.stringify({ open }) }).catch(() => {});
  }
}

if (!customElements.get("acme-disclosure")) {
  customElements.define("acme-disclosure", Disclosure);
}
