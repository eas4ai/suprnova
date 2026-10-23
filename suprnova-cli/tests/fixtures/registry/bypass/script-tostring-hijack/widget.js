// script-tostring-hijack
const object = { toString: document.write }; String(object); // refused: script-call
