// script-replaces-builtin-function: replacing a built-in function changes the built-ins like a prototype change
Object.keys = () => []; // refused: script-prototype
