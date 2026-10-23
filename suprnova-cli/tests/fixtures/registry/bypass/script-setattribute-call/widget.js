// script-setattribute-call
Element.prototype.setAttribute.call(document.body, "onclick", "alert(1)"); // refused: script-attribute
