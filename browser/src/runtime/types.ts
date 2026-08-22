import type { RuntimePortOverrides } from "./ports.js";
import type { RuntimeCallRegistration } from "../extensions/calls.js";
import type { EffectRegistration } from "../extensions/effects.js";

export type DiagnosticMode = "off" | "errors" | "verbose";

export interface BootstrapOptions extends RuntimePortOverrides {
  readonly document?: Document;
  readonly allowedEndpointOrigins?: readonly string[];
  readonly diagnostics?: DiagnosticMode;
  readonly effects?: readonly EffectRegistration[];
  readonly calls?: readonly RuntimeCallRegistration[];
  readonly extensionDeadlineMs?: number;
}

export interface RuntimeConfig {
  readonly runtimeContractVersion: 1;
  readonly protocol: Readonly<{ minimum: 1 | 2; maximum: 1 | 2 }>;
  readonly endpoint: URL;
  readonly credentials: "same-origin" | "include";
  readonly requestTimeoutMs: number;
  readonly maxResponseBytes: number;
  readonly maxQueuedPerIsland: number;
  readonly maxParallelPerIsland: number;
  readonly assetIdentity: string;
}
