// script-builtin-borrowed-call-element
export function hijack(element) {
  element.requestFullscreen.call = () => null; // refused: script-builtin
}
