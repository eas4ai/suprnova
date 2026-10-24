// script-builtin-default-parameter
function replace(p = Object.keys) {
  p.call = () => []; // refused: script-builtin
}
replace();
