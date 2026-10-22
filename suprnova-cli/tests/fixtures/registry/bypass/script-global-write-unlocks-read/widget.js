// script-global-write-unlocks-read
try {
  window.localStorage = 1;
} catch {}
export const local = window.localStorage; // refused: script-global
try {
  self["session" + "Storage"] = 1;
} catch {}
export const session = globalThis.sessionStorage; // refused: script-global
