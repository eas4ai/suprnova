// script-builtin-literal-name
const n = 0;
n.toPrecision.call = () => ""; // refused: script-builtin
