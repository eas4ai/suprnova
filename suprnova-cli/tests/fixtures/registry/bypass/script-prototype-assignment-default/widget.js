// script-prototype-assignment-default
let p;
[p = Array.prototype] = []; // refused: script-prototype
p.polluted = 1;
