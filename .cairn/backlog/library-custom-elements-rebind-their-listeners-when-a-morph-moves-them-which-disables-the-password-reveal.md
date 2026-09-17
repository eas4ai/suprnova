# Library custom elements rebind their listeners when a morph moves them, which disables the password reveal

Surfaced from: UI-018
Outside because: live-protocol-bounds bounds what one Live request and response carry (LIVE-028 to LIVE-030); how a vendored custom element survives a morph that moves it is not part of that work.
Captured: 2026-09-17T11:53:47.691Z

Found 2026-09-17 in the adversarial review of Live and the component library before 2.1.0. combobox.js, input-otp.js, date-picker.js and password-input.js add listeners and observers in connectedCallback with no guard and no disconnectedCallback; dialog.js, sheet.js and drawer.js guard with data-sn-ready on an unkeyed host whose attribute any morph removes, because the server markup lacks it. idiomorph 0.7.4 moves persistent-id nodes with moveBefore or insertBefore, and an element without connectedMoveCallback is disconnected and connected again, so connectedCallback runs twice. Reproduced in Chromium: after appendChild or moveBefore, one click on the password reveal toggles twice and the input stays a password field.
