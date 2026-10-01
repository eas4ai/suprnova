import { createPublicApi, type SuprnovaLivePublicApi } from "./bootstrap.js";

declare global {
  interface Window {
    readonly SuprnovaLive?: SuprnovaLivePublicApi;
  }
}

const current: unknown = Reflect.get(window, "SuprnovaLive");
if (current === undefined) {
  Object.defineProperty(window, "SuprnovaLive", {
    configurable: false,
    enumerable: true,
    value: createPublicApi(),
    writable: false,
  });
} else if (
  typeof current !== "object" ||
  current === null ||
  Reflect.get(current, "version") !== "0.1.0" ||
  Reflect.get(current, "runtimeContractVersion") !== 1 ||
  typeof Reflect.get(current, "boot") !== "function"
) {
  throw new Error("suprnova_live_global_conflict");
}

export {};
