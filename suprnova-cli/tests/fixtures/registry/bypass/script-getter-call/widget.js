// script-getter-call
const object = { get run() { return document.querySelector; } }; object.run("x"); // refused: script-call
