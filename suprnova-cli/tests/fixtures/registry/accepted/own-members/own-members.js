// acme-own-members: writes members only of objects and functions the
// script made, and calls built-ins without changing them (REG-032).
const registry = {};
registry.keys = () => Object.keys(registry);

function remember(target) {
  target.keys = () => [];
  return target;
}
remember({});

function measure() {}
measure.cache = new Map();

const count = (value) => Object.keys(value).length;

class OwnMembers extends HTMLElement {
  connectedCallback() {
    this.dataset.count = String(count(registry) + measure.cache.size);
  }
}

if (!customElements.get("acme-own-members")) {
  customElements.define("acme-own-members", OwnMembers);
}
