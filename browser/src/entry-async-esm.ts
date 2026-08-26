import { registerRuntimeFeature } from "./features/producer.js";
import { asyncFeature } from "./async-updates/feature.js";

export { asyncFeature };
export { AsyncDocumentOwner, configureAsync, createAsyncFeature } from "./async-updates/feature.js";
export {
  BrowserAsyncTransportPorts,
  OriginHandshakeScheduler,
} from "./async-updates/connections.js";
export type {
  AsyncAuthorityPort,
  AsyncAuthorizationRequest,
  AsyncAuthorizationResult,
  AsyncFeatureOptions,
  AsyncFreshnessObservation,
  AsyncFreshnessObserver,
} from "./async-updates/feature.js";
export type {
  AsyncTransportPorts,
  BrowserAsyncTransportOptions,
  DocumentTransportConnectRequest,
  DocumentTransportFailure,
  DocumentTransportPort,
} from "./async-updates/connections.js";
export type {
  AsyncTransportAuthorization,
  AuthorizedLogicalSubscription,
  DocumentTransportKey,
  StreamPosition,
  PollFallbackPolicy,
} from "./async-updates/types.js";
export type { PollEnvironment, PollPolicy, PollStatus } from "./async-updates/poll.js";
export const asyncRegistration = registerRuntimeFeature(globalThis, asyncFeature);

export default asyncFeature;
