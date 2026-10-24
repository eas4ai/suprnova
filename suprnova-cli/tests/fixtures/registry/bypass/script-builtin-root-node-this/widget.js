// script-builtin-root-node-this
export class Widget extends HTMLElement {
  connectedCallback() {
    this.getRootNode().open = () => null; // refused: script-builtin
  }
}
