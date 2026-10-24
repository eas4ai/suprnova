// script-destructuring-shorthand-url
let u = "/ok";
({ u } = { u: "https://evil.example/x" });
const img = new Image();
img.src = u; // refused: script-url
