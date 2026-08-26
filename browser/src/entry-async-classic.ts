import { registerClassicAsyncFeature } from "./features/producer.js";
import { asyncFeature, configureAsync } from "./async-updates/feature.js";

registerClassicAsyncFeature(globalThis, asyncFeature, configureAsync);
