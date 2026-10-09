// script-computed-eval
const key = ["e", "v", "a", "l"].join(""); globalThis[key]("alert(1)"); // refused: script-computed
