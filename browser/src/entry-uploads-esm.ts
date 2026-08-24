import { registerRuntimeFeature } from "./features/producer.js";
import { uploadsFeature } from "./features/unavailable-uploads.js";

export { uploadsFeature };
export const uploadsRegistration = registerRuntimeFeature(globalThis, uploadsFeature);

export default uploadsFeature;
