// script-prototype-member-parameter
function replace(p) {
  p.call = () => 1; // refused: script-prototype
}
replace(Array.prototype.slice);
