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

export function argumentsFrom(arguments_: readonly string[]): UploadBudgetArguments;

export function bundledModule(
  entryPoint: string,
  platformName: "browser" | "node",
  format: "esm" | "iife",
  globalName?: string,
): Promise<UploadBudgetBundle>;

export function measureRun(
  browser: Browser,
  artifactSource: string,
  workloadSource: string,
): Promise<unknown>;
