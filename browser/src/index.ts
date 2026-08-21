/** Version of the internal Live conformance package. */
export const ENGINE_VERSION = "0.1.0";

/** Snapshot schema versions understood by the iteration 001 package. */
export const SUPPORTED_SNAPSHOT_VERSIONS = [1] as const;

/** Wire protocol versions understood by the iteration 001 package. */
export const SUPPORTED_PROTOCOL_VERSIONS = [1] as const;

export {
  FIXTURE_FILES_V1,
  expectedFixtureManifestSha256,
  fixtureManifestSha256,
  loadFixtureSet,
} from "./conformance.js";
export { canonicalize, parseCanonicalJson } from "./canonical.js";
export { verifySnapshotFixture } from "./crypto.js";
export { applicationPlan } from "./ordering.js";
export { validateUpdateRequest, validateUpdateResponse } from "./protocol.js";
