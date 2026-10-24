// script-conditional-timer
(location.hash ? setTimeout : setInterval)("alert(1)", 10); // refused: script-timer
