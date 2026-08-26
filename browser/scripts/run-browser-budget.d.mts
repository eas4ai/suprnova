export interface BrowserBudgetArguments {
  readonly baseline: string;
  readonly output: string;
  readonly updateBaseline: boolean;
}

export function argumentsFrom(arguments_: readonly string[]): BrowserBudgetArguments;
