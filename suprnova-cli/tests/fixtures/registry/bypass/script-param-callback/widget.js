// script-param-callback
function invoke(fn) { fn("alert(1)"); } invoke(window.alert); // refused: script-call
