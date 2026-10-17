// script-setattribute-srcset
document.querySelector("img").setAttribute("srcset", "x.png, https://evil.example/t.png 2x"); // refused: script-url
