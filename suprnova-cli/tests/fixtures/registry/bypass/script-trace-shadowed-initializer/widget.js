// script-trace-shadowed-initializer
const y = "https://evil.example/x";
const x = y;
export function show() {
  const y = "/ok";
  const img = new Image();
  img.src = x; // refused: script-url
}
