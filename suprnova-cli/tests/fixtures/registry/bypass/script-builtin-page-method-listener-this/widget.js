// script-builtin-page-method-listener-this
export class Widget extends HTMLElement {
  connectedCallback() {
    document.addEventListener("click", this.onClick);
  }

  onClick() {
    this.createElement = () => null; // refused: script-builtin
  }
}
