export const ENGINE_VERSION = "0.1.0";
export const RUNTIME_CONTRACT_VERSION = 1 as const;
export const SUPPORTED_SNAPSHOT_VERSIONS = [1] as const;
export const SUPPORTED_PROTOCOL_VERSIONS = [1, 2] as const;

export type SupportedProtocolVersion = (typeof SUPPORTED_PROTOCOL_VERSIONS)[number];
