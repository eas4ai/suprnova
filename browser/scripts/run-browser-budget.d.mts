export interface BrowserBudgetArguments {
  readonly baseline: string;
  readonly dedicated: boolean;
  readonly idleMs: number;
  readonly output: string;
  readonly release: boolean;
  readonly runs: number;
  readonly samples: number;
  readonly warmups: number;
}

export function argumentsFrom(arguments_: readonly string[]): BrowserBudgetArguments;
