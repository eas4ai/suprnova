// suprnova.tabs - local tab selection and arrow keys (NAV-002). This
// element answers a click or an arrow key inside its tablist by moving
// aria-selected, the roving tabindex and the panels' hidden attribute;
// every one of those sits on a keyed, preserved root the browser owns
// across a morph, and nothing here talks to the server. Light DOM: the
// tabs and panels are the server's markup. Tabs nest: an instance acts only
// on the tabs and panels whose nearest sn-tabs is itself, and leaves the
// events that bubble out of a nested instance to that instance (NAV-007).
if (!customElements.get("sn-tabs")) {
  customElements.define(
    "sn-tabs",
    class extends HTMLElement {
      connectedCallback() {
        this.addEventListener("click", this.#onClick);
        this.addEventListener("keydown", this.#onKeydown);
      }

      disconnectedCallback() {
        this.removeEventListener("click", this.#onClick);
        this.removeEventListener("keydown", this.#onKeydown);
      }

      // A morph that moves the element keeps its listeners.
      connectedMoveCallback() {}

      #owns(element) {
        return element.closest("sn-tabs") === this;
      }

      #tabs() {
        return [...this.querySelectorAll('[role="tab"]')].filter((tab) => this.#owns(tab));
      }

      #select(tab, focus) {
        for (const candidate of this.#tabs()) {
          const selected = candidate === tab;
          candidate.setAttribute("aria-selected", selected ? "true" : "false");
          if (selected) candidate.removeAttribute("tabindex");
          else candidate.setAttribute("tabindex", "-1");
          const panel = this.querySelector(`#${CSS.escape(candidate.getAttribute("aria-controls") ?? "")}`);
          if (panel && this.#owns(panel)) panel.hidden = !selected;
        }
        if (focus) tab.focus();
      }

      #onClick = (event) => {
        const tab = event.target instanceof Element ? event.target.closest('[role="tab"]') : null;
        if (tab && this.#owns(tab)) this.#select(tab, false);
      };

      #onKeydown = (event) => {
        const current = event.target instanceof Element ? event.target.closest('[role="tab"]') : null;
        if (!current || !this.#owns(current)) return;
        const tabs = this.#tabs();
        const index = tabs.indexOf(current);
        let next = null;
        if (event.key === "ArrowRight") next = tabs[(index + 1) % tabs.length];
        else if (event.key === "ArrowLeft") next = tabs[(index - 1 + tabs.length) % tabs.length];
        else if (event.key === "Home") next = tabs[0];
        else if (event.key === "End") next = tabs[tabs.length - 1];
        if (!next) return;
        event.preventDefault();
        this.#select(next, true);
      };
    },
  );
}
