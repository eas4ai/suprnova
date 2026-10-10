// script-setattribute-imagesrcset
document.querySelector("img").setAttribute("imagesrcset", "x.png 1x, //evil.example/t.png 2x"); // refused: script-url
