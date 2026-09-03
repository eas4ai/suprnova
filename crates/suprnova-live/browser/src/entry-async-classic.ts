import { registerClassicAsyncFeature } from "./features/producer.js";
import { asyncFeature, configureAsync } from "./async-updates/feature.js";
import { browserAsyncOptions } from "./async-updates/browser-host.js";

registerClassicAsyncFeature(globalThis, asyncFeature, configureAsync);

// The framework's classic boot configures the feature with the default browser
// host when this global is present; applications may configure it themselves
// before boot instead.
Reflect.set(
  globalThis,
  "SuprnovaLiveAsync",
  Object.freeze({ browserOptions: browserAsyncOptions }),
);
