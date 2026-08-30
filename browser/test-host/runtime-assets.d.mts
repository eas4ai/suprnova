export interface ValidatedRuntimeAssets {
  readonly artifactRoot: string;
  readonly manifest: Readonly<Record<string, unknown>>;
}

export function validateRuntimeAssets(root: string): Promise<ValidatedRuntimeAssets>;

export function afterRuntimeAssetsValidated<T>(
  root: string,
  start: (artifacts: ValidatedRuntimeAssets) => T | Promise<T>,
): Promise<T>;
