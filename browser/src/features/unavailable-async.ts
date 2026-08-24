import {
  defineAsyncFeature,
  type RuntimeFeature,
  type RuntimeFeatureDefinition,
} from "./contract.js";

const unavailableAsync: RuntimeFeatureDefinition = Object.freeze({
  connectDocument() {
    return Object.freeze({
      connectIsland() {
        return undefined;
      },
      dispose() {
        // The shared-foundation artifact is inert until async behavior lands.
      },
    });
  },
});

export const asyncFeature: RuntimeFeature = defineAsyncFeature(unavailableAsync);
