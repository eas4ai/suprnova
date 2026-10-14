// script-builtin-borrowed-call
Math.random().toPrecision.call = () => ""; // refused: script-builtin
