export interface AsyncBudgetArguments {
  readonly artifact: string | null;
  readonly baseline: string;
  readonly child: boolean;
  readonly output: string;
  readonly profile: "qualified" | "reduced";
  readonly recordExploratory: boolean;
  readonly retentionMutation:
    | "large_island_buffer"
    | "none"
    | "predecessor_transport"
    | "stale_current_payload"
    | "stale_queued_payload";
  readonly serverOutput: string;
  readonly verifyRetentionMutations: boolean;
}

export class AsyncBudgetRunnerError extends Error {
  readonly code: string;
}

export function argumentsFrom(arguments_: readonly string[]): AsyncBudgetArguments;

export function childExecutionFailure(
  execution: Readonly<{
    error?: Readonly<{ code?: string }>;
    status: number | null;
  }>,
  watchdogCode: string,
  failureCode: string,
): string | null;

export function exactServerEvidence(
  value: unknown,
  artifactSha256: string,
): Readonly<Record<string, unknown>>;

export function verifyArtifactBinding(
  artifactBytes: Uint8Array,
  manifestBytes: Uint8Array,
): Readonly<{ manifestSha256: string; sha256: string }>;
