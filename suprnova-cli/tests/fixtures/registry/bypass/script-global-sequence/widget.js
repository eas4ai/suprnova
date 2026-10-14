// script-global-sequence
export const viaSequence = (0, window).localStorage; // refused: script-global
export function viaConditional(flag) {
  return (flag ? window : self).localStorage; // refused: script-global
}
export function viaLogical(w) {
  return (w || globalThis).localStorage; // refused: script-global
}
