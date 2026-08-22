import { parseRuntimeConfig } from "./runtime/config.js";
import { RuntimeDiagnostics } from "./runtime/diagnostics.js";
import {
  productionRuntimePorts,
  resolveRuntimePorts,
  type RuntimePortOverrides,
} from "./runtime/ports.js";
import { SuprnovaLiveRuntime, type RuntimeHandle, type RuntimeStatus } from "./runtime/runtime.js";
import type { BootstrapOptions } from "./runtime/types.js";
import {
  ENGINE_VERSION,
  RUNTIME_CONTRACT_VERSION,
  SUPPORTED_PROTOCOL_VERSIONS,
} from "./version.js";

export const RUNTIME_SYMBOL = Symbol.for("suprnova.live.runtime.v1");

interface RuntimeHost extends Window {
  readonly [RUNTIME_SYMBOL]?: RuntimeHandle;
}

export interface SuprnovaLivePublicApi {
  readonly version: typeof ENGINE_VERSION;
  readonly runtimeContractVersion: typeof RUNTIME_CONTRACT_VERSION;
  readonly supportedProtocolVersions: typeof SUPPORTED_PROTOCOL_VERSIONS;
  boot(options?: BootstrapOptions): RuntimeHandle;
}

function runtimeWindow(document: Document): Window {
  const window = document.defaultView;
  if (window === null) throw new Error("runtime_window_unavailable");
  return window;
}

function portOverrides(options: BootstrapOptions): RuntimePortOverrides {
  return {
    ...(options.clock === undefined ? {} : { clock: options.clock }),
    ...(options.randomness === undefined ? {} : { randomness: options.randomness }),
    ...(options.transport === undefined ? {} : { transport: options.transport }),
    ...(options.navigation === undefined ? {} : { navigation: options.navigation }),
    ...(options.observers === undefined ? {} : { observers: options.observers }),
    ...(options.scheduler === undefined ? {} : { scheduler: options.scheduler }),
    ...(options.features === undefined ? {} : { features: options.features }),
  };
}

function existingRuntime(host: RuntimeHost): RuntimeHandle | null {
  const value: unknown = Reflect.get(host, RUNTIME_SYMBOL);
  if (value === undefined) return null;
  if (
    typeof value === "object" &&
    value !== null &&
    typeof Reflect.get(value, "status") === "function" &&
    typeof Reflect.get(value, "stop") === "function" &&
    typeof Reflect.get(value, "runEffect") === "function" &&
    typeof Reflect.get(value, "call") === "function"
  ) {
    return value as RuntimeHandle;
  }
  throw new Error("runtime_symbol_conflict");
}

export function boot(options: BootstrapOptions = {}): RuntimeHandle {
  const document = options.document ?? globalThis.document;
  const window = runtimeWindow(document);
  const host = window as RuntimeHost;
  const existing = existingRuntime(host);
  if (existing !== null) return existing;

  const config = parseRuntimeConfig(document, options);
  const diagnostics = new RuntimeDiagnostics({ mode: options.diagnostics ?? "errors" });
  const ports = resolveRuntimePorts(productionRuntimePorts(window), portOverrides(options));
  const runtime = new SuprnovaLiveRuntime({
    document,
    config,
    diagnostics,
    ports,
    ...(options.effects === undefined ? {} : { effects: options.effects }),
    ...(options.calls === undefined ? {} : { calls: options.calls }),
    ...(options.extensionDeadlineMs === undefined
      ? {}
      : { extensionDeadlineMs: options.extensionDeadlineMs }),
  });
  Object.defineProperty(host, RUNTIME_SYMBOL, {
    configurable: false,
    enumerable: false,
    value: runtime,
    writable: false,
  });
  return runtime;
}

export function createPublicApi(): SuprnovaLivePublicApi {
  return Object.freeze({
    version: ENGINE_VERSION,
    runtimeContractVersion: RUNTIME_CONTRACT_VERSION,
    supportedProtocolVersions: SUPPORTED_PROTOCOL_VERSIONS,
    boot,
  });
}

export type { BootstrapOptions, RuntimeHandle, RuntimeStatus };
