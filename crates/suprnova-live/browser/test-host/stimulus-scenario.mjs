/* global document, HTMLElement, MutationObserver, window */

import { Application, Controller } from "/test-vendor/stimulus.js";
import { boot } from "/assets/suprnova-live.esm.js";
import { installStimulusAdapter } from "/test-boot/features.js";

installStimulusAdapter(window);

const counts = new Map();
const waiters = new Set();

function counter(name) {
  const current = counts.get(name) ?? { connect: 0, disconnect: 0 };
  counts.set(name, current);
  return current;
}

function notifyLifecycleChange() {
  for (const waiter of [...waiters]) {
    if (!waiter.predicate()) continue;
    waiters.delete(waiter);
    waiter.resolve();
  }
}

function lifecycleState(predicate) {
  if (predicate()) return Promise.resolve();
  return new Promise((resolve) => {
    waiters.add({ predicate, resolve });
  });
}

class ProbeController extends Controller {
  connect() {
    counter(this.element.dataset.probe).connect += 1;
    notifyLifecycleChange();
    if (this.element.hasAttribute("data-probe-throw")) {
      throw new Error("test controller failure");
    }
  }

  disconnect() {
    counter(this.element.dataset.probe).disconnect += 1;
    notifyLifecycleChange();
  }
}

let errors = 0;
const application = new Application(document.documentElement);
application.handleError = () => {
  errors += 1;
  notifyLifecycleChange();
};
const observer = new MutationObserver(notifyLifecycleChange);
observer.observe(document.documentElement, { attributes: true, subtree: true });
const runtime = boot({
  stimulus: {
    application,
    definitions: [{ identifier: "probe", controllerConstructor: ProbeController }],
  },
});
let stopped = false;

try {
  if (document.readyState === "loading") {
    await new Promise((resolve) => {
      document.addEventListener("DOMContentLoaded", resolve, { once: true });
    });
  }
  await lifecycleState(() => counter("preserved").connect === 1 && errors === 1);

  const morphComplete = lifecycleState(
    () =>
      document.querySelector("#stimulus-preserved")?.getAttribute("data-state") === "morphed" &&
      counter("removed").disconnect === 1 &&
      counter("inserted").connect === 1,
  );
  const action = document.querySelector("#stimulus-action");
  if (!(action instanceof HTMLElement)) throw new Error("stimulus_action_missing");
  action.click();
  await morphComplete;
  if (counter("preserved").connect !== 1 || counter("preserved").disconnect !== 0) {
    throw new Error("preserved controller duplicated");
  }

  document.documentElement.dataset.stimulusRuntimeAfterError =
    document.querySelector("#stimulus-island")?.getAttribute("data-suprnova-live-status") ??
    "missing";
  const disposalComplete = lifecycleState(
    () =>
      counter("preserved").disconnect === 1 &&
      counter("inserted").disconnect === 1 &&
      counter("detached").disconnect === 1 &&
      counter("nested").disconnect === 1,
  );
  runtime.stop();
  stopped = true;
  await disposalComplete;

  for (const name of ["preserved", "removed", "inserted", "detached", "nested"]) {
    const value = counter(name);
    document.documentElement.dataset[`stimulus${name[0].toUpperCase()}${name.slice(1)}`] =
      `${value.connect}:${value.disconnect}`;
  }
  document.documentElement.dataset.stimulusDisposal = "complete";
  document.documentElement.dataset.stimulusErrors = String(errors);
  document.documentElement.dataset.stimulusReady = "true";
} finally {
  observer.disconnect();
  if (!stopped) runtime.stop();
}
