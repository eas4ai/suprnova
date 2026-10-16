// Another class defines a method named like a browser API; this one
// calls the browser's.
class Helper {
  setAttribute() {}
}

class Evil extends HTMLElement {
  connectedCallback() {
    this.setAttribute("onclick", "alert(1)"); // refused: script-attribute
  }
}

new Helper().setAttribute();
customElements.define("evil-widget", Evil);
