// script-this-global
(function () { return this; })().eval("alert(1)"); // refused: script-this
