import { registerClassicFeature } from "./features/producer.js";
import { uploadsFeature } from "./uploads/feature.js";

registerClassicFeature(globalThis, uploadsFeature);
