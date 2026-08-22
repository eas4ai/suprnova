import {
  ENGINE_VERSION,
  RUNTIME_CONTRACT_VERSION,
  SUPPORTED_PROTOCOL_VERSIONS,
  SUPPORTED_SNAPSHOT_VERSIONS,
} from "./version.js";

export const PRODUCTION_CACHE_CONTROL = "public, max-age=31536000, immutable" as const;
export const PRODUCTION_CONTENT_TYPE = "text/javascript; charset=utf-8" as const;
export const REPRODUCIBLE_BUILD_TIMESTAMP = "1970-01-01T00:00:00.000Z" as const;

export interface RuntimeAsset {
  readonly file: string;
  readonly bytes: number;
  readonly sha256: string;
  readonly sri: `sha256-${string}`;
  readonly content_type: typeof PRODUCTION_CONTENT_TYPE;
  readonly script_kind: "module" | "classic";
  readonly preload_rel: "modulepreload" | "preload";
  readonly cache_control: typeof PRODUCTION_CACHE_CONTROL;
}

export interface RuntimeAssetManifest {
  readonly schema_version: 1;
  readonly engine_version: typeof ENGINE_VERSION;
  readonly runtime_contract_version: typeof RUNTIME_CONTRACT_VERSION;
  readonly protocol_versions: typeof SUPPORTED_PROTOCOL_VERSIONS;
  readonly snapshot_versions: typeof SUPPORTED_SNAPSHOT_VERSIONS;
  readonly built_at: typeof REPRODUCIBLE_BUILD_TIMESTAMP;
  readonly assets: readonly RuntimeAsset[];
  readonly provenance: {
    readonly idiomorph: {
      readonly name: "idiomorph";
      readonly version: "0.7.4";
      readonly license: "0BSD";
      readonly bundled: true;
    };
  };
}
