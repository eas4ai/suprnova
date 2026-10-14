// script-builtin-destructured
const { keys } = Object; // refused: script-builtin
keys.call = () => [];
