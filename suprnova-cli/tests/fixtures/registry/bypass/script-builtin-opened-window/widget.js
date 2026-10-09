// script-builtin-opened-window
window.open("/x").JSON.parse = () => ({}); // refused: script-builtin
