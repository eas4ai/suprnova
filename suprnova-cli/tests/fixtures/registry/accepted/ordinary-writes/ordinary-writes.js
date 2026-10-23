// acme-ordinary-writes: the writes and reads an ordinary component makes on
// the page, its elements and values it builds, which the built-in rule
// leaves admitted (REG-032).
const o = {};
o.call = 1;

const s = new Set();
s.add(1);

const label = (key) => key.toUpperCase();
const cells = [];

export const keys = (x) => Object.keys(x).forEach(label);
export const items = (text) => JSON.parse(text).items;
export const now = () => new Date().getTime();
export const count = [1, 2].map(label).length;
export const size = "abc".length + [o, s][0].call;

class OrdinaryWrites extends HTMLElement {
  state = { open: false };

  connectedCallback() {
    this.querySelector(".x").textContent = "y";
    const input = this.shadowRoot.querySelector("input");
    if (input) this.shadowRoot.querySelector("input").value = "";
    this.state.open = true;
    const el = this.querySelector(".toggle");
    el.classList.add("on");
    el.addEventListener("input", (event) => {
      this.dataset.value = event.target.value;
    });
    this.getRootNode().title = "x";
    cells[0] = el;
    cells[0].hidden = false;
  }
}

if (!customElements.get("acme-ordinary-writes")) {
  customElements.define("acme-ordinary-writes", OrdinaryWrites);
}
