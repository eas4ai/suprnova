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
} from "./async-updates/feature.js";
export type {
  AsyncTransportPorts,
  BrowserAsyncTransportOptions,
  DocumentTransportPort,
} from "./async-updates/connections.js";
export const asyncRegistration = registerRuntimeFeature(globalThis, asyncFeature);

export default asyncFeature;
