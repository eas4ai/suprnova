import type { RuntimePortOverrides } from "./ports.js";

export type DiagnosticMode = "off" | "errors" | "verbose";

export interface BootstrapOptions extends RuntimePortOverrides {
  readonly document?: Document;
  readonly allowedEndpointOrigins?: readonly string[];
  readonly diagnostics?: DiagnosticMode;
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
