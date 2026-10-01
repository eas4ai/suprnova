export interface UploadServerProcessInvocation {
  readonly cpuSet: string;
  readonly profile: "qualified" | "reduced";
  readonly resultPath: string;
  readonly runIndex: number;
}

export interface CollectUploadServerRunsOptions {
  readonly baseline?: string | null;
  readonly cpuSet?: string;
  readonly destination: string;
  readonly profile: "qualified" | "reduced";
  readonly runProcess?: (invocation: UploadServerProcessInvocation) => Promise<void>;
}

export function collectUploadServerRuns(options: CollectUploadServerRunsOptions): Promise<void>;
