// script-builtin-parameter
function replace(p) {
  p.call = () => []; // refused: script-builtin
}
replace(Object.keys);
