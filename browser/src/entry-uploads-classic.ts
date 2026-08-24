import { registerClassicFeature } from "./features/producer.js";
import { uploadsFeature } from "./features/unavailable-uploads.js";

registerClassicFeature(globalThis, uploadsFeature);
