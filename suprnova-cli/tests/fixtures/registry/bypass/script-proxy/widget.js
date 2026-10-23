// script-proxy
new Proxy({}, { get: () => 1 }); // refused: script-global
