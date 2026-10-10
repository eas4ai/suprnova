// script-async-constructor
Object.getPrototypeOf(async function () {}).constructor("alert(1)")(); // refused: script-eval
