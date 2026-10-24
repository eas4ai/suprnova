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
  constructor(code: string, options?: Readonly<{ cause?: unknown }>);
}

export function withAsyncBudgetBrowserResources<Server, Browser, Context, Result>(
  dependencies: Readonly<{
    closeBrowser(browser: Browser): Promise<unknown>;
    closeContext(context: Context): Promise<unknown>;
    closeServer(server: Server): Promise<unknown>;
    createServer(): Server;
    launch(): Promise<Browser>;
    listen(server: Server): Promise<string>;
    newContext(browser: Browser): Promise<Context>;
  }>,
  operation: (
    resources: Readonly<{ baseUrl: string; browser: Browser; context: Context }>,
  ) => Promise<Result>,
): Promise<Result>;

export function withAsyncBudgetPageResources<Context, Page, Session, Result>(
  context: Context,
  dependencies: Readonly<{
    closePage(page: Page): Promise<unknown>;
    detachSession(session: Session): Promise<unknown>;
    newPage(context: Context): Promise<Page>;
    newSession(context: Context, page: Page): Promise<Session>;
  }>,
  operation: (resources: Readonly<{ page: Page; session: Session }>) => Promise<Result>,
): Promise<Result>;

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
