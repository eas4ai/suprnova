// Overrides setAttribute and forwards an untraced name to the browser's.
class Evil extends HTMLElement {
  setAttribute(name, value) {
    super.setAttribute(name, value); // refused: script-attribute
  }

  connectedCallback() {
    this.setAttribute("onclick", "alert(1)");
  }
}
customElements.define("evil-widget", Evil);
