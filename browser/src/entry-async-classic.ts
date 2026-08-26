import { registerClassicFeature } from "./features/producer.js";
import { asyncFeature } from "./async-updates/feature.js";

registerClassicFeature(globalThis, asyncFeature);
