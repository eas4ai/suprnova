// script-prototype-parameter
function pollute(p) {
  p.polluted = 1;
}
pollute(Array.prototype); // refused: script-prototype
