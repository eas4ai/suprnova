// script-builtin-handed-back
Promise.resolve(Math).then((m) => { // refused: script-builtin
  m.random = () => 0;
});
