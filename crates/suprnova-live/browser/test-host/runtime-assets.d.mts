export interface ValidatedRuntimeAssets {
  readonly artifactRoot: string;
  asset(file: string): Readonly<{
    bytes: Uint8Array;
    cacheControl: string;
    contentType: string;
  }> | null;
}

export function validateRuntimeAssets(root: string): Promise<ValidatedRuntimeAssets>;

export function afterRuntimeAssetsValidated<T>(
  root: string,
  start: (artifacts: ValidatedRuntimeAssets) => T | Promise<T>,
): Promise<T>;
