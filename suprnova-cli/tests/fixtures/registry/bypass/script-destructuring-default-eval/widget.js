// script-destructuring-default-eval
let a;
[a = eval("alert(1)")] = []; // refused: script-eval
