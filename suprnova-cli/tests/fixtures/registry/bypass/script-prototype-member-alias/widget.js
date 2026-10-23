// script-prototype-member-alias
const s = Array.prototype.slice;
s.call = () => 1; // refused: script-prototype
