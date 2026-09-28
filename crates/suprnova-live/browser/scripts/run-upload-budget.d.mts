import type { Browser } from "@playwright/test";

export interface UploadBudgetArguments {
  readonly baseline: string;
  readonly output: string;
  readonly profile: "qualified" | "reduced";
  readonly recordExploratory: boolean;
  readonly serverResult: string;
}

export interface UploadBudgetBundle {
  readonly inputs: readonly string[];
  readonly source: string;
}

export interface AtomicWriteEvidenceOptions {
  readonly failStage?: "after_partial_write" | "before_rename" | "none";
}

export function argumentsFrom(arguments_: readonly string[]): UploadBudgetArguments;

export function bundledModule(
  entryPoint: string,
  platformName: "browser" | "node",
  format: "esm" | "iife",
  globalName?: string,
): Promise<UploadBudgetBundle>;

export function atomicWriteEvidence(
  destination: string,
  contents: string | Uint8Array,
  protectedPath: string | null,
  options?: AtomicWriteEvidenceOptions,
): Promise<void>;

export function measureRun(
  browser: Browser,
  artifactSource: string,
  workloadSource: string,
  options?: Readonly<{ watchdogMilliseconds?: number }>,
): Promise<unknown>;
