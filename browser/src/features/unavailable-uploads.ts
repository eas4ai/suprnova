import {
  defineUploadsFeature,
  type RuntimeFeature,
  type RuntimeFeatureDefinition,
} from "./contract.js";

const unavailableUploads: RuntimeFeatureDefinition = Object.freeze({
  connectDocument() {
    return Object.freeze({
      connectIsland() {
        return undefined;
      },
      dispose() {
        // The shared-foundation artifact is inert until upload behavior lands.
      },
    });
  },
});

export const uploadsFeature: RuntimeFeature = defineUploadsFeature(unavailableUploads);
