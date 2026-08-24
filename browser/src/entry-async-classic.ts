import { registerClassicFeature } from "./features/producer.js";
import { asyncFeature } from "./features/unavailable-async.js";

registerClassicFeature(globalThis, asyncFeature);
