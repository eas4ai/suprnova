// script-builtin-opened-window-name
const popup = window.open("/x");
popup.Object.keys = () => []; // refused: script-builtin
