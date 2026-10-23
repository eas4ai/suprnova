// script-reflect-alias
const R = globalThis.Reflect; R.get(window, "eval"); // refused: script-property
