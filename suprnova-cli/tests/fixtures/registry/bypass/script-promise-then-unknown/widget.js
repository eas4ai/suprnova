// script-promise-then-unknown
Promise.resolve("x").then(document.body.requestFullscreen.bind(document.body)); // refused: script-call
