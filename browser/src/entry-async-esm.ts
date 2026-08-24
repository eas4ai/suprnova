import { registerRuntimeFeature } from "./features/producer.js";
import { asyncFeature } from "./features/unavailable-async.js";

export { asyncFeature };
export const asyncRegistration = registerRuntimeFeature(globalThis, asyncFeature);

export default asyncFeature;
