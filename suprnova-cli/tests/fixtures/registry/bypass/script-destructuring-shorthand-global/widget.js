// script-destructuring-shorthand-global
({ location } = { location: "javascript:alert(1)" }); // refused: script-url
({ onerror } = { onerror: () => 1 }); // refused: script-global
