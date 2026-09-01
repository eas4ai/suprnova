import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";

import { buildSync } from "esbuild";

const instanceEnvelope = {
  body: {
    build_id: "build-2026-08-21",
    component: {
      contract_digest: "ICEiIyQlJicoKSorLC0uLzAxMjM0NTY3ODk6Ozw9Pj8",
      memo_schema_version: 1,
      mount_schema_version: 1,
      name: "catalog.search",
      state_schema_version: 1,
    },
    expires_at: "2000",
    extensions: {},
    form: "instance",
    instance_id: "sLGys7S1tre4ubq7vL2-vw",
    issued_at: "1000",
    key_id: "snapshot-v1",
    memo: { page: 1 },
    revision: "7",
    route: "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA",
    schema_version: 1,
    scope: "kJGSk5SVlpeYmZqbnJ2en6ChoqOkpaanqKmqq6ytrq8",
    slot: "search-results",
    state: { query: "rust", selected: "1" },
  },
  signature: "pA88mZ0Hd4jb9jTqvrNfrwpMD4pkIB74XfHiOOhCpzE",
};

const seedEnvelope = {
  body: {
    advisory_generations: [],
    build_id: "build-2026-08-21",
    component: {
      contract_digest: "ICEiIyQlJicoKSorLC0uLzAxMjM0NTY3ODk6Ozw9Pj8",
      memo_schema_version: 1,
      mount_schema_version: 1,
      name: "catalog.search",
      state_schema_version: 1,
    },
    extensions: {},
    form: "seed",
    issued_at: "1000",
    key_id: "snapshot-v1",
    max_age_ms: "500",
    memo: { page: 1 },
    mount: { catalog: "primary" },
    refresh_on_promote: false,
    route: "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA",
    schema_version: 1,
    slot: "search-results",
    state: { query: "rust", selected: "1" },
  },
  signature: "T5Wga8fVi-Dl7Lj2JDOpcbAHbBm6snERYFCo4m82Uc0",
};

const bootSource = 'import { boot } from "/assets/suprnova-live.esm.js"; boot();';
const externalModuleBuild = buildSync({
  bundle: true,
  format: "esm",
  legalComments: "none",
  minify: true,
  platform: "browser",
  stdin: {
    contents: 'import { boot } from "./src/bootstrap.ts"; boot();',
    resolveDir: new URL("../", import.meta.url).pathname,
    sourcefile: "suprnova-live-test-module.ts",
  },
  target: ["chrome111", "edge111", "firefox128", "safari16.4"],
  write: false,
});
const externalModuleFile = externalModuleBuild.outputFiles?.[0];
if (externalModuleFile === undefined) throw new Error("module_test_asset_missing");
export const externalModuleBootSource = externalModuleFile.text;
export const externalClassicBootSource = "window.SuprnovaLive.boot();\n";
const optionalDriverBuild = buildSync({
  bundle: true,
  format: "esm",
  legalComments: "none",
  minify: true,
  platform: "browser",
  stdin: {
    contents: 'export { installStimulusAdapter } from "./src/features/stimulus.ts";',
    resolveDir: new URL("../", import.meta.url).pathname,
    sourcefile: "suprnova-live-test-feature-driver.ts",
  },
  target: ["chrome111", "edge111", "firefox128", "safari16.4"],
  write: false,
});
const optionalDriverFile = optionalDriverBuild.outputFiles?.[0];
if (optionalDriverFile === undefined) throw new Error("feature_driver_test_asset_missing");
export const optionalDriverSource = optionalDriverFile.text;

function sha256(value) {
  return createHash("sha256").update(value).digest("base64");
}

function integrity(value) {
  return `sha256-${sha256(value)}`;
}

const externalModuleIntegrity = "sha256-zYoww/Aib+RNCJSlDhnNeiAqjmLZHWkXX4Qbhpr8f3w=";
const externalClassicBootIntegrity = "sha256-driX1AsbsALchFYpBEj6JN/QRgsB3x5rHdMifbdcfOA=";
const externalClassicRuntimeIntegrity = "sha256-6G53OzNWd7paFjvWKRJQSYkyMoHyIlNmSoE0Vm9CH3Y=";

function requireReviewedIntegrity(value, expected, name) {
  if (integrity(value) !== expected) throw new Error(`${name}_integrity_drift`);
}

function externalModuleScript(variant = "plain") {
  if (variant === "plain") {
    return '<script type="module" src="/test-boot/module.js"></script>';
  }
  if (variant === "nonce") {
    return '<script type="module" src="/test-boot/module.js" nonce="suprnova-test"></script>';
  }
  if (variant === "integrity") {
    requireReviewedIntegrity(externalModuleBootSource, externalModuleIntegrity, "module_boot");
    return '<script type="module" src="/test-boot/module.js" integrity="sha256-zYoww/Aib+RNCJSlDhnNeiAqjmLZHWkXX4Qbhpr8f3w=" crossorigin="anonymous"></script>';
  }
  throw new Error("unsupported_external_module_script_variant");
}

function externalClassicScripts(variant = "plain") {
  if (variant === "plain") {
    return '<script src="/assets/suprnova-live.classic.js"></script><script src="/test-boot/classic.js"></script>';
  }
  if (variant === "nonce") {
    return '<script src="/assets/suprnova-live.classic.js" nonce="suprnova-test"></script><script src="/test-boot/classic.js" nonce="suprnova-test"></script>';
  }
  throw new Error("unsupported_external_classic_script_variant");
}

function hashOnlyModulePolicy() {
  requireReviewedIntegrity(externalModuleBootSource, externalModuleIntegrity, "module_boot");
  return {
    "content-security-policy": `default-src 'none'; script-src '${externalModuleIntegrity}' 'strict-dynamic'; connect-src 'self'`,
  };
}

function hashOnlyClassicPolicy() {
  const runtime = readFileSync(new URL("../dist/suprnova-live.classic.js", import.meta.url));
  requireReviewedIntegrity(runtime, externalClassicRuntimeIntegrity, "classic_runtime");
  requireReviewedIntegrity(externalClassicBootSource, externalClassicBootIntegrity, "classic_boot");
  return {
    "content-security-policy": `default-src 'none'; script-src '${externalClassicRuntimeIntegrity}' '${externalClassicBootIntegrity}'; connect-src 'self'`,
  };
}

function hashOnlyClassicDocument() {
  const runtime = readFileSync(new URL("../dist/suprnova-live.classic.js", import.meta.url));
  requireReviewedIntegrity(runtime, externalClassicRuntimeIntegrity, "classic_runtime");
  requireReviewedIntegrity(externalClassicBootSource, externalClassicBootIntegrity, "classic_boot");
  return document(
    island(),
    '<script src="/assets/suprnova-live.classic.js" integrity="sha256-6G53OzNWd7paFjvWKRJQSYkyMoHyIlNmSoE0Vm9CH3Y=" crossorigin="anonymous"></script><script src="/test-boot/classic.js" integrity="sha256-driX1AsbsALchFYpBEj6JN/QRgsB3x5rHdMifbdcfOA=" crossorigin="anonymous"></script>',
  );
}

function hostileScenario(mode, overrides = {}) {
  return document(
    island({
      protocolMinimum: "2",
      body: '<button id="hostile-action" live:click.prevent="save">Exercise hostile response</button><p id="hostile-original">Last accepted hostile fixture</p>',
    }),
    moduleBoot(),
    { endpoint: `/live?mode=${mode}`, ...overrides },
  );
}

function nestedMarkup(depth, body) {
  let result = body;
  for (let level = 0; level < depth; level += 1) result = `<div>${result}</div>`;
  return result;
}

function hostileInitialLimits() {
  const attributes = Array.from({ length: 257 }, (_, index) => `data-hostile-${index}="x"`).join(
    " ",
  );
  const elements = Array.from({ length: 4_100 }, (_, index) => {
    if (index === 4_099) {
      return `<button id="hostile-over-limit" type="button" live:click.prevent="save">${index}</button>`;
    }
    return `<button type="button" live:click.prevent="save">${index}</button>`;
  }).join("");
  const text = "x".repeat(1_048_577);
  return document(
    island({
      body: `<p id="hostile-limit-marker">Hostile limits stay visible</p>${nestedMarkup(129, `<div ${attributes}>${elements}${text}</div>`)}`,
    }),
    moduleBoot(),
  );
}

function accessibilityScenario() {
  return document(
    island({
      rootAttributes: ' live:signal="open:false,second:false"',
      body: `<h1>Accessible Live controls</h1>
        <button id="a11y-disclosure" type="button" aria-controls="a11y-panel" aria-expanded="false" live:toggle="open" live:expanded="open">Details</button>
        <section id="a11y-panel" hidden aria-hidden="true" inert live:show="open"><h2>Details panel</h2><p>Progressively enhanced content.</p></section>
        <div role="tablist" aria-label="Examples">
            <button id="a11y-tab-first" type="button" role="tab" aria-selected="false" aria-controls="a11y-tabpanel-first" tabindex="-1">First</button>
          <button id="a11y-tab-second" type="button" role="tab" aria-selected="false" aria-controls="a11y-tabpanel-second" live:toggle="second" live:selected="second">Second</button>
        </div>
          <section id="a11y-tabpanel-first" role="tabpanel" aria-labelledby="a11y-tab-first" hidden aria-hidden="true" inert>First panel</section>
        <section id="a11y-tabpanel-second" role="tabpanel" aria-labelledby="a11y-tab-second" hidden aria-hidden="true" inert live:show="second">Second panel</section>
        <form action="/scenario/navigationDestination" method="get">
          <label for="a11y-name">Name</label><input id="a11y-name" name="name" aria-invalid="true" aria-describedby="a11y-error">
          <p id="a11y-error" role="alert">Name is required.</p>
          <button type="submit">Submit normally</button>
        </form>
        <output id="a11y-live" aria-live="polite">Ready</output>
        <div id="a11y-busy" aria-busy="true">Saving</div>
        <button id="a11y-disabled" type="button" disabled>Unavailable</button>
        <div id="a11y-inert" inert><button type="button">Inert action</button></div>
        <a id="a11y-fallback" href="/scenario/navigationDestination">Ordinary fallback</a>`,
    }),
    moduleBoot(),
  );
}

function fullFlowScenario() {
  return document(
    `${island({
      protocolMinimum: "2",
      rootAttributes: ' live:signal="open:false"',
      body: `<button id="flow-disclosure" type="button" live:toggle="open">Show local panel</button>
        <section id="flow-panel" hidden aria-hidden="true" inert live:show="open">Local panel</section>
        <label>Query <input id="flow-model" live:model.action="query"></label>
        <button id="flow-action" type="button" live:click.prevent="search" live:loading.disabled="search">Run server action</button>`,
    })}<a id="flow-native-link" href="/scenario/navigationDestination">Continue with native navigation</a>`,
    moduleBoot(),
    { endpoint: "/live?mode=full-flow" },
  );
}

function escapeAttribute(value) {
  return value.replaceAll("&", "&amp;").replaceAll('"', "&quot;");
}

function encodedEnvelope(envelope) {
  return Buffer.from(JSON.stringify(envelope), "utf8").toString("base64url");
}

export function island({
  body,
  documentKey = "primary",
  envelope = instanceEnvelope,
  form = "instance",
  instanceId = "sLGys7S1tre4ubq7vL2-vw",
  protocolMinimum = "1",
  revision = form === "seed" ? "0" : "7",
  rootAttributes = "",
  slot = "search-results",
  content = "Server-rendered search results",
} = {}) {
  const instance = form === "instance" ? ` data-suprnova-live-instance="${instanceId}"` : "";
  return `<section data-suprnova-live-root="${slot}" data-suprnova-live-island data-suprnova-live-component="catalog.search" data-suprnova-live-slot="${slot}" data-suprnova-live-document-key="${documentKey}" data-suprnova-live-protocol-min="${protocolMinimum}" data-suprnova-live-contract="1" data-suprnova-live-snapshot-kind="${form}" data-suprnova-live-snapshot="${escapeAttribute(encodedEnvelope(envelope))}" data-suprnova-live-revision="${revision}" data-suprnova-live-lazy-complete="false"${instance}${rootAttributes}>${body ?? `<p>${content}</p>`}</section>`;
}

export function morphChild(content = "Nested original") {
  return island({
    documentKey: "morph-child",
    envelope: {
      ...instanceEnvelope,
      body: {
        ...instanceEnvelope.body,
        instance_id: "EBESExQVFhcYGRobHB0eHw",
        slot: "morph-child-slot",
      },
    },
    instanceId: "EBESExQVFhcYGRobHB0eHw",
    protocolMinimum: "1",
    slot: "morph-child-slot",
    content,
  });
}

export function stimulusChild() {
  return island({
    body: '<div id="stimulus-nested" data-controller="probe" data-probe="nested" data-suprnova-live-key="nested"></div>',
    documentKey: "stimulus-child",
    envelope: {
      ...instanceEnvelope,
      body: {
        ...instanceEnvelope.body,
        instance_id: "EBESExQVFhcYGRobHB0eHw",
        slot: "stimulus-child-slot",
      },
    },
    instanceId: "EBESExQVFhcYGRobHB0eHw",
    slot: "stimulus-child-slot",
  });
}

function config(overrides = {}) {
  return `<script id="suprnova-live-config" type="application/json">${JSON.stringify({
    asset_identity: "suprnova-live-test-v1",
    credentials: "same-origin",
    endpoint: "/live",
    max_parallel_per_island: 1,
    max_queued_per_island: 8,
    max_response_bytes: 1_048_576,
    protocol: { maximum: 2, minimum: 1 },
    request_timeout_ms: 5_000,
    runtime_contract_version: 1,
    ...overrides,
  })}</script>`;
}

function document(body, scripts = "", configOverrides = {}) {
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Suprnova Live conformance</title></head><body>${config(configOverrides)}<main>${body}</main>${scripts}</body></html>`;
}

function moduleBoot(attributes = "") {
  if (attributes === "") {
    return '<script type="module">import { boot } from "/assets/suprnova-live.esm.js"; boot();</script>';
  }
  if (attributes === ' nonce="suprnova-test"') {
    return '<script type="module" nonce="suprnova-test">import { boot } from "/assets/suprnova-live.esm.js"; boot();</script>';
  }
  throw new Error("unsupported_module_boot_attributes");
}

function extensionBoot() {
  return `<script type="module">
    import { boot } from "/assets/suprnova-live.esm.js";
    const messageSchema = { type: "object", properties: { message: { type: "string", maxBytes: 64 } }, required: ["message"], additionalProperties: false };
    const booleanSchema = { type: "boolean" };
      const runtime = boot({
      effects: [{
        name: "announce",
        version: 1,
        schema: messageSchema,
        phase: "after_commit",
        run(context, payload) {
          const output = document.querySelector("#effect-output");
          if (output !== null) output.textContent = payload.message;
          document.documentElement.dataset.effectContext = Object.keys(context).sort().join(",");
          document.documentElement.dataset.effectMutation = [
            Reflect.set(context, "snapshot", {}),
            Reflect.set(context.island, "revision", 9),
          ].join(":");
        },
      }],
      calls: [{
        name: "mark-ready",
        input: booleanSchema,
        output: booleanSchema,
        run(context, input) { return context.local("ready", input); },
      }],
      });
      Reflect.set(window, "__suprnovaExtensionRuntime", runtime);
    const root = document.querySelector("[data-suprnova-live-island]");
    const call = document.querySelector("#extension-call");
    if (root === null || call === null) throw new Error("missing_extension_fixture");
    const effect = await runtime.runEffect(root, { name: "announce", payload: { message: "effect-ready" } });
    const callResult = await runtime.call(call, "mark-ready", true);
    const wrongScope = await runtime.runEffect(call, { name: "announce", payload: { message: "wrong" } });
    let forgedCall = "accepted";
    try { await runtime.call(root, "mark-ready", true); } catch { forgedCall = "rejected"; }
    document.documentElement.dataset.effectStatus = effect.status;
    document.documentElement.dataset.callResult = String(callResult);
    document.documentElement.dataset.wrongScope = wrongScope.status;
    document.documentElement.dataset.forgedCall = forgedCall;
    document.documentElement.dataset.extensionsReady = "true";
  </script>`;
}

function responseOrderBoot() {
  return `<script type="module">
    import { boot } from "/assets/suprnova-live.esm.js";
    const trace = [];
    window.__suprnovaResponseTrace = trace;
    document.addEventListener("suprnova:saved", () => trace.push("event"));
    boot({
      navigation: {
        assign(target) {
          trace.push("navigate");
          document.documentElement.dataset.navigationTarget = target.pathname + target.search;
        },
        replace() { trace.push("replace"); },
        reload() { trace.push("reload"); },
      },
      effects: [{
        name: "probe",
        version: 1,
        schema: { type: "object", properties: {}, required: [], additionalProperties: false },
        phase: "after_commit",
        run() { trace.push("effect"); },
      }],
    });
  </script>`;
}

function morphFailureBoot() {
  return `<script type="module">
    import { boot } from "/assets/suprnova-live.esm.js";
    boot({
      navigation: {
        assign(target) {
          document.documentElement.dataset.morphRecovery = target.pathname;
        },
        replace() {},
        reload() {
          document.documentElement.dataset.morphRecovery = "reload";
        },
      },
    });
  </script>`;
}

function transitionBoot(disableAnimations = false) {
  const disableAnimationsScript = disableAnimations
    ? '<script>Object.defineProperty(Element.prototype, "getAnimations", { configurable: true, value: undefined });</script>'
    : "";
  return `${disableAnimationsScript}<script type="module">
    const trace = [];
    window.__suprnovaTransitionTrace = trace;
    new MutationObserver((records) => {
      for (const record of records) {
        const value = record.target.getAttribute("data-suprnova-live-transition-state");
        if (value !== null) trace.push(record.target.id + ":" + value);
      }
    }).observe(document.documentElement, {
      attributeFilter: ["data-suprnova-live-transition-state"],
      attributes: true,
      subtree: true,
    });
    import { boot } from "/assets/suprnova-live.esm.js"; boot();
  </script>`;
}

export function transitionBody(revision = "7") {
  if (revision === "7") {
    return `<button id="transition-action" live:click.prevent="save">Run transitions</button>
      <div id="transition-list">
        <div id="transition-leave" data-suprnova-live-key="leave" live:transition.leave="fade">Leave</div>
        <div id="transition-move" data-suprnova-live-key="move" live:transition.both="fade">Move</div>
        <div id="transition-anchor" data-suprnova-live-key="anchor">Anchor</div>
      </div>
      <div id="transition-state" data-suprnova-live-key="state" live:transition.both="fade">Before</div>`;
  }
  return `<button id="transition-action" live:click.prevent="save">Run transitions</button>
    <div id="transition-list">
      <div id="transition-enter" data-suprnova-live-key="enter" live:transition.enter="fade">Enter</div>
      <div id="transition-anchor" data-suprnova-live-key="anchor">Anchor</div>
      <div id="transition-move" data-suprnova-live-key="move" live:transition.both="fade">Move</div>
    </div>
    <div id="transition-state" data-suprnova-live-key="state" live:transition.both="fade">After</div>`;
}

function transitionScenario(disableAnimations = false) {
  const style = `<style>
    @keyframes suprnova-live-test-motion { from { opacity: 0.2; } to { opacity: 1; } }
    .suprnova-live-transition { animation: suprnova-live-test-motion 80ms linear; }
  </style>`;
  return document(
    `${style}${island({ body: transitionBody(), protocolMinimum: "2" })}`,
    transitionBoot(disableAnimations),
    { endpoint: "/live?mode=transitions" },
  );
}

function stimulusBoot() {
  return '<script type="module" src="/scenario/stimulus-driver.js"></script>';
}

function preservationBoot() {
  return `<script type="module">
    import { Application, Controller } from "/test-vendor/stimulus.js";
    import { boot } from "/assets/suprnova-live.esm.js";
    import { installStimulusAdapter } from "/test-boot/features.js";

    installStimulusAdapter(window);

    const lifecycle = { connect: 0, disconnect: 0 };
    const expose = () => {
      document.documentElement.dataset.replaceLifecycle = lifecycle.connect + ":" + lifecycle.disconnect;
    };
    class ReplacementController extends Controller {
      connect() { lifecycle.connect += 1; expose(); }
      disconnect() { lifecycle.disconnect += 1; expose(); }
    }
    const application = new Application(document.documentElement);
    boot({
      stimulus: {
        application,
        definitions: [{ identifier: "replacement", controllerConstructor: ReplacementController }],
      },
    });
  </script>`;
}

export function preservationBody(revision = "7") {
  const removesControlledNodes = revision === "9";
  const persisted = `<article id="persisted-panel" data-suprnova-live-key="persisted" live:persist="layout" live:signal="open:false">
    <button id="persisted-toggle" type="button" live:toggle="open">Toggle persisted state</button>
    <span id="persisted-state" hidden aria-hidden="true" inert live:show="open">Persisted ${revision}</span>
  </article>`;
  const origin = revision === "8" ? "" : persisted;
  const destination = revision === "8" ? persisted : "";
  return `<button id="preservation-action" live:click.prevent="save">Morph controls</button>
    ${
      removesControlledNodes
        ? ""
        : `<section id="preserved-panel" data-suprnova-live-key="preserved" live:preserve.self data-owner="${revision === "7" ? "browser" : `server-${revision}`}">
            <span id="preserved-child">${revision === "7" ? "Initial child" : `Server child ${revision}`}</span>
          </section>`
    }
    <section id="ignored-children" data-suprnova-live-key="ignored-children" live:ignore.children data-state="${revision === "7" ? "initial" : `server-${revision}`}">
      <span id="ignored-child">Server-owned child ${revision}</span>
    </section>
    <section id="ignored-subtree" data-suprnova-live-key="ignored-subtree" live:ignore.subtree data-state="${revision === "7" ? "browser" : `server-${revision}`}">
      <span id="ignored-subtree-child">Server-owned subtree ${revision}</span>
    </section>
    <section id="replaced-panel" data-controller="replacement" data-suprnova-live-key="replaced" live:replace.subtree data-generation="${revision}">Replacement ${revision}</section>
    <div id="persist-origin">${origin}</div>
    <div id="persist-destination">${destination}</div>
    ${
      removesControlledNodes
        ? ""
        : `<section id="teleported-dialog" role="dialog" aria-labelledby="teleported-title" data-suprnova-live-key="teleported" live:teleport="#modal-root">
            <h2 id="teleported-title">Dialog ${revision}</h2>
            <button id="teleported-focus" type="button">Keep focus</button>
          </section>`
    }`;
}

function continuityBoot() {
  return `<script type="module">
    import { Application, Controller } from "/test-vendor/stimulus.js";
    import { boot } from "/assets/suprnova-live.esm.js";
    import { installStimulusAdapter } from "/test-boot/features.js";

    installStimulusAdapter(window);

    const lifecycle = { connect: {}, disconnect: {} };
    const expose = () => {
      document.documentElement.dataset.continuityLifecycle = JSON.stringify(lifecycle);
    };
    class ContinuityController extends Controller {
      connect() {
        const name = this.element.getAttribute("data-probe") || "unknown";
        lifecycle.connect[name] = (lifecycle.connect[name] || 0) + 1;
        expose();
      }
      disconnect() {
        const name = this.element.getAttribute("data-probe") || "unknown";
        lifecycle.disconnect[name] = (lifecycle.disconnect[name] || 0) + 1;
        expose();
      }
    }
    const application = new Application(document.documentElement);
    boot({
      stimulus: {
        application,
        definitions: [{ identifier: "continuity", controllerConstructor: ContinuityController }],
      },
    });
  </script>`;
}

export function continuityBody(revision = "7") {
  const numericRevision = Number(revision);
  const resetSignals = numericRevision >= 9;
  const signalKey = resetSignals ? "signal-reset" : "signal-scope";
  const signalDeclaration = resetSignals ? "open:false" : "open:false";
  const focused =
    revision === "9"
      ? ""
      : '<input id="continuity-focused" data-suprnova-live-key="focused" aria-label="Focus target" value="focus me">';
  const defaultFocused =
    numericRevision >= 10
      ? ""
      : '<button id="continuity-default-focused" type="button" data-suprnova-live-key="default-focused">Default fallback source</button>';
  const declaredFallback =
    numericRevision >= 10
      ? ""
      : '<button id="continuity-fallback" type="button" data-suprnova-live-key="focus-fallback" data-suprnova-live-focus-fallback>Focus fallback</button>';
  const controllerRemoved =
    revision === "7"
      ? '<div id="controller-removed" data-controller="continuity" data-probe="removed" data-suprnova-live-key="controller-removed"></div>'
      : "";
  const controllerInserted =
    revision === "7"
      ? ""
      : '<div id="controller-inserted" data-controller="continuity" data-probe="inserted" data-suprnova-live-key="controller-inserted"></div>';
  const order =
    revision === "7"
      ? `${focused}<span data-suprnova-live-key="order-label">Before</span>`
      : `<span data-suprnova-live-key="order-label">After ${revision}</span>${focused}`;
  return `<button id="continuity-action" live:click.prevent="save">Morph continuity</button>
    <div id="continuity-order">${order}</div>
    ${defaultFocused}
    ${declaredFallback}
    <label>Text <input id="continuity-text" data-suprnova-live-key="text" value="server-${revision}" live:model.action="text"></label>
    <label>Correction <input id="continuity-correction" data-suprnova-live-key="correction" value="${revision === "7" ? "original" : `corrected-${revision}`}"${revision === "7" ? "" : ` data-suprnova-live-authoritative="${revision}"`}></label>
    <label>Check <input id="continuity-check" data-suprnova-live-key="check" type="checkbox"></label>
    <fieldset>
      <legend>Radio</legend>
      <label>A <input id="continuity-radio-a" data-suprnova-live-key="radio-a" type="radio" name="continuity-radio" value="a" checked></label>
      <label>B <input id="continuity-radio-b" data-suprnova-live-key="radio-b" type="radio" name="continuity-radio" value="b"></label>
    </fieldset>
    <label>Select <select id="continuity-select" data-suprnova-live-key="select"><option value="a" selected>A</option><option value="b">B</option></select></label>
    <label>Multiple <select id="continuity-multiple" data-suprnova-live-key="multiple" multiple><option value="a" selected>A</option><option value="b">B</option><option value="c">C</option></select></label>
    <label>File <input id="continuity-file" data-suprnova-live-key="file" type="file"></label>
    <input id="continuity-selection" data-suprnova-live-key="selection" aria-label="Selection continuity" value="selection-value-${revision}">
    <div id="continuity-editable" data-suprnova-live-key="editable" contenteditable="true" role="textbox" aria-label="Editable continuity">editable value ${revision}</div>
    <div id="continuity-scroll" data-suprnova-live-key="scroll" data-suprnova-live-scroll role="region" aria-label="Continuity scroll region" tabindex="0" style="height: 50px; overflow: auto">
      <div style="height: 400px">Scrollable ${revision}</div>
    </div>
    ${
      numericRevision >= 10
        ? ""
        : `<div id="continuity-signal" data-suprnova-live-key="${signalKey}" live:signal="${signalDeclaration}">
      <button id="continuity-toggle" type="button" live:toggle="open">Toggle signal</button>
      <span id="continuity-signal-state" hidden aria-hidden="true" inert live:show="open">Open ${revision}</span>
    </div>`
    }
    <div id="controller-preserved" data-controller="continuity" data-probe="preserved" data-suprnova-live-key="controller-preserved" data-revision="${revision}"></div>
    ${controllerRemoved}
    ${controllerInserted}`;
}

function hashPolicy() {
  const digest = createHash("sha256").update(bootSource).digest("base64");
  return `default-src 'none'; script-src 'self' 'sha256-${digest}'; connect-src 'self'`;
}

function navigationBoot({ captureFailure = false, unsupported = false } = {}) {
  const captureFailureScript = captureFailure
    ? '<script>const originalSetProperty = CSSStyleDeclaration.prototype.setProperty; CSSStyleDeclaration.prototype.setProperty = function(name, value, priority) { if (name === "view-transition-name") throw new Error("capture failed"); return originalSetProperty.call(this, name, value, priority); };</script>'
    : "";
  const unsupportedScript = unsupported
    ? '<script>Object.defineProperty(document, "startViewTransition", { configurable: true, value: undefined });</script>'
    : "";
  return `${captureFailureScript}${unsupportedScript}<script>
    document.documentElement.dataset.documentToken = crypto.randomUUID();
    document.addEventListener("input", (event) => {
      if (event.target?.id === "dirty-input") {
        event.target.closest("[data-suprnova-live-navigation-guard]")?.setAttribute("data-suprnova-live-dirty", "true");
      }
    });
  </script>${moduleBoot()}`;
}

function navigationSource() {
  return document(
    `<h1>Navigation source</h1>
      <p id="source-marker">Complete source document</p>
      <a id="ordinary-link" href="/scenario/navigationDestination">Ordinary destination</a>
      <a id="redirect-link" href="/navigation/redirect">Redirect destination</a>
      <a id="error-link" href="/scenario/navigationError">Error document</a>
      <a id="fragment-link" href="/scenario/navigationDestination#fragment-target">Fragment destination</a>
      <a id="same-fragment-link" href="#source-marker">Same-document fragment</a>
      <a id="external-link" href="https://example.invalid/">External destination</a>
      <a id="new-tab-link" href="/scenario/navigationDestination?tab=1" target="_blank">New tab destination</a>
      <a id="download-link" href="/navigation/download" download="report.txt">Download</a>
      <form id="get-form" action="/scenario/navigationDestination" method="get">
        <label>Query <input name="query" value="forms"></label>
        <button type="submit">GET destination</button>
      </form>
      <form id="post-form" action="/navigation/post" method="post">
        <label>Message <input name="message" value="posted"></label>
        <button type="submit">POST destination</button>
      </form>
      <section id="dirty-scope" data-suprnova-live-navigation-guard="Discard the unsaved navigation draft?">
        <label>Unsaved draft <input id="dirty-input"></label>
        <a id="guarded-link" href="/scenario/navigationDestination?guarded=1">Guarded destination</a>
      </section>
      <a id="eligible-prefetch" href="/scenario/navigationPrefetch" live:prefetch.eager data-suprnova-live-prefetch-cache="public">Eligible prefetch</a>
      <a id="private-prefetch" href="/scenario/navigationPrivate" live:prefetch.eager data-suprnova-live-prefetch-cache="private">Private prefetch</a>
      <a id="hidden-prefetch" href="/scenario/navigationHidden" live:prefetch.eager data-suprnova-live-prefetch-cache="public" hidden>Hidden prefetch</a>`,
    navigationBoot(),
  );
}

function navigationDestination(label = "Navigation destination") {
  return document(
    `<h1 id="destination-focus" tabindex="-1" data-suprnova-live-document-focus>${label}</h1>
      <p id="destination-marker">Complete canonical destination</p>
      <div style="height: 1000px"></div>
      <h2 id="fragment-target" tabindex="-1">Fragment target</h2>
      <a id="return-link" href="/scenario/navigation">Return to source</a>`,
    navigationBoot(),
  );
}

function documentTransitionSource({ captureFailure = false, unsupported = false } = {}) {
  return document(
    `<style>@view-transition { navigation: auto; }</style>
      <h1>Document transition source</h1>
      <div id="transition-hero" data-suprnova-live-document-transition="hero">Hero source</div>
      <a id="document-transition-link" href="/scenario/documentTransitionDestination" live:navigate.transition data-suprnova-live-transition-name="document">Transition destination</a>
      <section id="transition-dirty" data-suprnova-live-navigation-guard="Discard the unsaved transition draft?">
        <label>Unsaved transition draft <input id="dirty-input"></label>
        <a id="cancel-transition-link" href="/scenario/documentTransitionDestination?guarded=1" live:navigate.transition data-suprnova-live-transition-name="document">Guarded transition</a>
      </section>`,
    navigationBoot({ captureFailure, unsupported }),
  );
}

function lifecycleBoot() {
  return `<script type="module">
    import { boot } from "/assets/suprnova-live.esm.js";
    if (!("onfreeze" in document)) {
      Object.defineProperty(document, "onfreeze", { configurable: true, value: null, writable: true });
    }
    if (!("onresume" in document)) {
      Object.defineProperty(document, "onresume", { configurable: true, value: null, writable: true });
    }
    const runtime = boot();
    const token = crypto.randomUUID();
    let boots = 1;
    document.documentElement.dataset.lifecycleToken = token;
    document.documentElement.dataset.lifecycleBoots = String(boots);
    Reflect.set(window, "__suprnovaLifecycleProbe", {
      runtime,
      bootAgain() {
        const same = boot() === runtime;
        boots += 1;
        document.documentElement.dataset.lifecycleBoots = String(boots);
        return same;
      },
    });
  </script>`;
}

function lifecycleScenario() {
  return document(
    island({
      body: `<button id="lifecycle-action" live:click.prevent="save">Run delayed action</button>
        <p id="lifecycle-content">Lifecycle original</p>`,
      protocolMinimum: "2",
    }),
    lifecycleBoot(),
    { endpoint: "/live?mode=lifecycle" },
  );
}

function asyncLifecycleScenario() {
  return document(
    `${island({
      protocolMinimum: "2",
      rootAttributes:
        ' live:stream="orders" live:signal="open:false" aria-busy="false" data-live-stream-state="disconnected" data-live-stream-motion="allowed"',
      body: `<h1>Async order updates</h1>
        <p data-live-stream-status aria-label="Order updates">Updates disconnected</p>
        <p id="async-content">Server-rendered async content</p>
        <output id="async-effect-count" aria-label="Applied async effects">0</output>
        <button id="keep-focus" type="button">Keep focus</button>
        <button id="degrade-stream" type="button">Degrade stream</button>
        <button id="reconnect-stream" type="button">Reconnect stream</button>
        <button id="close-stream" type="button">Close stream</button>
        <button id="replace-island" type="button" live:click.prevent="replace_stream">Replace island contents</button>
        <button id="run-live-action" type="button" live:click.prevent="save">Run Live action</button>
        <output id="async-action-result"></output>
        <button id="local-toggle" type="button" live:toggle="open">Local details</button>
        <p id="local-panel" hidden aria-hidden="true" inert live:show="open">Local signal remains available</p>`,
    })}
      <button id="remove-island" type="button">Remove island</button>
      <form action="/navigation/post" method="post"><label>Native value <input name="value"></label><button type="submit">Submit normally</button></form>
      <a href="/scenario/lifecycleDestination">Native destination</a>`,
    '<script type="module" nonce="suprnova-async-test" src="/test-async/lifecycle.js"></script>',
    { endpoint: "http://127.0.0.1:4174/live" },
  );
}

export function uploadBody(replacement = false) {
  const suffix = replacement ? "-replacement" : "";
  const keySuffix = replacement ? "-replacement" : "-stable";
  return `<label for="attachment-input${suffix}">Attachment</label>
    <input id="attachment-input${suffix}" type="file" live:upload="attachment" data-suprnova-live-key="attachment-input${keySuffix}">
    <div id="attachment-progress${suffix}" live:progress="attachment" data-suprnova-live-key="attachment-progress${keySuffix}" aria-label="Attachment upload progress" aria-errormessage="attachment-error${suffix}"></div>
    <p id="attachment-error${suffix}" hidden>Attachment upload failed.</p>
    <button id="attachment-cancel${suffix}" type="button" live:upload.cancel="attachment" data-suprnova-live-key="attachment-cancel${keySuffix}">Cancel upload</button>
    <button id="attachment-retry${suffix}" type="button" live:upload.retry="attachment" data-suprnova-live-key="attachment-retry${keySuffix}">Retry upload</button>
    <button id="attachment-remove${suffix}" type="button" live:upload.remove="attachment" data-suprnova-live-key="attachment-remove${keySuffix}">Remove upload</button>
    <button id="attachment-morph" type="button" live:click.prevent="save" data-suprnova-live-key="attachment-morph">Morph upload form</button>`;
}

function uploadsBoot() {
  return `<script type="module" nonce="suprnova-upload-test">
    import { configureUploads, uploadsRegistration } from "/assets/suprnova-live.uploads.esm.js";
    import { boot } from "/assets/suprnova-live.esm.js";

    const cspViolations = [];
    window.__uploadCspViolations = cspViolations;
    window.__uploadRegistration = uploadsRegistration;
    document.addEventListener("securitypolicyviolation", (event) => {
      cspViolations.push(event.violatedDirective);
    });

    let revision = 0;
    let releaseChunk = null;
    const fixtureGrant = ["browser", "fixture", "grant"].join("-");
    class FixtureUploadTransport {
      async send(request) {
        revision += 1;
        if (request.operation === "create") {
          return {
            grant: fixtureGrant,
            handle: "018f47c1-2af0-7cc4-a001-000000000001",
            revision: String(revision),
            state: "queued",
          };
        }
        if (request.operation === "put_chunk") {
          await new Promise((resolve) => {
            releaseChunk = resolve;
            window.__releaseUploadChunk = () => {
              const release = releaseChunk;
              releaseChunk = null;
              if (release !== null) release();
            };
          });
          return { revision: String(revision), state: "transferring" };
        }
        if (request.operation === "complete") {
          return { revision: String(revision), state: "ready" };
        }
        if (request.operation === "cancel") {
          return { revision: String(revision), state: "canceled" };
        }
        return { nextChunkIndex: 0, revision: String(revision), state: "transferring" };
      }
    }
    configureUploads({
      chunkBytes: 256 * 1024,
      maxActive: 1,
      maxItems: 8,
      maxQueueBytes: 256 * 1024,
      randomness: {
        next: 0,
        idempotencyKey() {
          this.next += 1;
          return "browser-fixture-" + String(this.next);
        },
      },
      transport: new FixtureUploadTransport(),
    });
    const input = document.querySelector("#attachment-input");
    if (!(input instanceof HTMLInputElement)) throw new Error("upload_input_missing");
    const valueDescriptor = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value");
    const filesDescriptor = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "files");
    if (valueDescriptor?.get === undefined || valueDescriptor.set === undefined || filesDescriptor?.get === undefined || filesDescriptor.set === undefined) {
      throw new Error("upload_input_descriptor_missing");
    }
    const valueWrites = [];
    const filesWrites = [];
    window.__uploadInitialInput = input;
    window.__uploadValueWrites = valueWrites;
    window.__uploadFilesWrites = filesWrites;
    input.addEventListener("change", (event) => {
      window.__uploadChangeTrusted = event.isTrusted;
    });
    Object.defineProperty(input, "value", {
      configurable: true,
      get() { return Reflect.apply(valueDescriptor.get, this, []); },
      set(value) {
        valueWrites.push(value);
        Reflect.apply(valueDescriptor.set, this, [value]);
      },
    });
    Object.defineProperty(input, "files", {
      configurable: true,
      get() { return Reflect.apply(filesDescriptor.get, this, []); },
      set(value) {
        filesWrites.push(value);
        Reflect.apply(filesDescriptor.set, this, [value]);
      },
    });
    boot();
    document.documentElement.dataset.uploadRuntime = "ready";
  </script>`;
}

function uploadsScenario() {
  return document(island({ body: uploadBody(), protocolMinimum: "2" }), uploadsBoot(), {
    endpoint: "/live?mode=uploads-morph",
  });
}

function iteration004UploadControls() {
  return `<label for="iteration-upload">Iteration 004 file</label>
    <input id="iteration-upload" type="file" live:upload="attachment" data-suprnova-live-key="iteration-upload" aria-describedby="iteration-upload-error">
    <div id="iteration-upload-progress" live:progress="attachment" data-suprnova-live-key="iteration-upload-progress" role="progressbar" aria-label="Iteration 004 upload progress" aria-errormessage="iteration-upload-error" aria-valuemin="0" aria-valuemax="100" aria-valuenow="0"></div>
    <p id="iteration-upload-error" role="alert">Upload failed. Retry or remove this file.</p>
    <button type="button" live:upload.cancel="attachment">Cancel upload</button>
    <button type="button" live:upload.retry="attachment">Retry upload</button>
    <button type="button" live:upload.remove="attachment">Remove upload</button>`;
}

function iteration004Scenario(searchParams = new URLSearchParams()) {
  const features = ["core", "uploads", "async", "both"].includes(searchParams.get("features"))
    ? searchParams.get("features")
    : "core";
  const format = searchParams.get("format") === "classic" ? "classic" : "esm";
  const artifact = ["missing", "incompatible"].includes(searchParams.get("artifact"))
    ? searchParams.get("artifact")
    : "current";
  const uploadArtifact = ["missing", "incompatible"].includes(searchParams.get("upload-artifact"))
    ? searchParams.get("upload-artifact")
    : artifact;
  const asyncArtifact = ["missing", "incompatible"].includes(searchParams.get("async-artifact"))
    ? searchParams.get("async-artifact")
    : artifact;
  const transport = searchParams.get("transport") === "websocket" ? "websocket" : "sse";
  const islands = searchParams.get("islands") === "2" ? 2 : 1;
  const lifecycle = searchParams.get("lifecycle") === "true";
  const hybrid = searchParams.get("hybrid") === "true";
  const controlledClock = searchParams.get("controlled-clock") === "true";
  const controlledUploadClock = searchParams.get("controlled-upload-clock") === "true";
  const uploadChunkBytes = searchParams.get("upload-chunk-bytes") === "262145" ? 262145 : 262144;
  const rejectUploadOnce = searchParams.get("upload-reject-once") === "true";
  const syntheticLifecycle = searchParams.get("synthetic-lifecycle") === "true";
  const hasUploads = features === "uploads" || features === "both";
  const hasAsync = features === "async" || features === "both";
  const rootAttributes = hasAsync
    ? ` live:stream${hybrid ? ".hybrid" : ""}="orders" live:signal="details:false" aria-busy="false" data-live-stream-state="disconnected" data-live-stream-motion="allowed"`
    : ' live:signal="details:false"';
  const primaryBody = `<h1>Iteration 004 integration</h1>
    <button type="button" live:toggle="details">Toggle local details</button>
    <p hidden aria-hidden="true" inert live:show="details">Local details are available</p>
    <details id="native-disclosure"><summary>Native disclosure</summary><p>Native fallback details</p></details>
    ${hasAsync ? '<p data-live-stream-status aria-label="Order updates">Updates disconnected</p>' : ""}
    ${hasUploads ? iteration004UploadControls() : ""}`;
  const primary = island({
    body: primaryBody,
    documentKey: "iteration-004-primary",
    protocolMinimum: "2",
    rootAttributes,
  });
  const secondary =
    hasAsync && islands === 2
      ? island({
          body: '<h2>Second stream island</h2><p data-live-stream-status aria-label="Second order updates">Updates disconnected</p>',
          documentKey: "iteration-004-secondary",
          envelope: {
            ...instanceEnvelope,
            body: {
              ...instanceEnvelope.body,
              instance_id: "EBESExQVFhcYGRobHB0eHw",
              slot: "iteration-004-secondary",
            },
          },
          instanceId: "EBESExQVFhcYGRobHB0eHw",
          protocolMinimum: "2",
          rootAttributes,
          slot: "iteration-004-secondary",
        })
      : "";
  const incompatibleClassicArtifact = (slot) => {
    if (slot === "uploads") {
      return '<script src="/scenario/iteration004-incompatible-feature.js" data-feature-slot="uploads"></script>';
    }
    if (slot === "async") {
      return '<script src="/scenario/iteration004-incompatible-feature.js" data-feature-slot="async"></script>';
    }
    throw new Error("unsupported_incompatible_feature_slot");
  };
  const classicFeatures = `${
    hasUploads && uploadArtifact === "current"
      ? '<script src="/suprnova-live.uploads.classic.js"></script>'
      : ""
  }${
    hasAsync && asyncArtifact === "current"
      ? '<script src="/suprnova-live.async.classic.js"></script>'
      : ""
  }${
    hasUploads && uploadArtifact === "incompatible" ? incompatibleClassicArtifact("uploads") : ""
  }${hasAsync && asyncArtifact === "incompatible" ? incompatibleClassicArtifact("async") : ""}`;
  const scripts = `<link rel="stylesheet" href="/scenario/iteration004.css">${
    format === "classic"
      ? `<script src="/scenario/iteration004-classic-registration-probe.js"></script>${classicFeatures}<script src="/suprnova-live.classic.js"></script>`
      : ""
  }<script type="module" src="/scenario/iteration004-driver.js"></script>`;
  const page = document(
    `${primary}${secondary}
      ${secondary.length === 0 ? "" : '<button id="remove-second-island" type="button">Remove second island</button>'}
      <a href="/scenario/iteration004Destination">Ordinary destination</a>
      <form action="/scenario/iteration004Destination" method="get"><button type="submit">Continue ordinarily</button></form>`,
    scripts,
    { endpoint: "/__live/async/poll" },
  );
  return page.replace(
    '<html lang="en">',
    `<html lang="en" data-iteration-004-features="${features}" data-iteration-004-format="${format}" data-iteration-004-upload-artifact="${uploadArtifact}" data-iteration-004-async-artifact="${asyncArtifact}" data-iteration-004-transport="${transport}" data-iteration-004-lifecycle="${String(lifecycle)}" data-iteration-004-synthetic-lifecycle="${String(syntheticLifecycle)}" data-iteration-004-controlled-clock="${String(controlledClock)}" data-iteration-004-controlled-upload-clock="${String(controlledUploadClock)}" data-iteration-004-upload-chunk-bytes="${uploadChunkBytes}" data-iteration-004-reject-upload-once="${String(rejectUploadOnce)}">`,
  );
}

export const scenarios = Object.freeze({
  accessibility: { html: accessibilityScenario() },
  asyncLifecycle: {
    headers: {
      "cache-control": "private, max-age=60",
      "content-security-policy":
        "default-src 'none'; script-src 'self' 'nonce-suprnova-async-test'; connect-src 'self' http://127.0.0.1:4174; style-src 'self' 'unsafe-inline'; form-action 'self'; base-uri 'none'",
    },
    html: asyncLifecycleScenario(),
  },
  fullFlow: { html: fullFlowScenario() },
  iteration004: {
    headers: {
      "cache-control": "private, max-age=60",
      "content-security-policy":
        "default-src 'none'; script-src 'self'; connect-src 'self' https://uploads.example.test; img-src 'self' data:; style-src 'self'; form-action 'self'; base-uri 'none'",
    },
    html: iteration004Scenario,
  },
  iteration004Destination: {
    headers: { "cache-control": "private, max-age=60" },
    html: document(
      '<h1>Iteration 004 destination</h1><a href="/scenario/iteration004?features=async&format=esm&transport=sse&lifecycle=true">Return to integration</a>',
    ),
  },
  hostileMalformedUtf8: { html: hostileScenario("hostile-malformed-utf8") },
  hostileHugeJson: { html: hostileScenario("hostile-huge-json") },
  hostilePrototypeKey: { html: hostileScenario("hostile-prototype-key") },
  hostileExtremeMorph: {
    html: hostileScenario("hostile-extreme-morph", { max_response_bytes: 4_194_304 }),
  },
  hostileDuplicateIdentity: { html: hostileScenario("hostile-duplicate-identity") },
  hostileInitialLimits: { html: hostileInitialLimits() },
  lifecycle: { html: lifecycleScenario() },
  uploads: {
    headers: {
      "content-security-policy":
        "default-src 'none'; script-src 'self' 'nonce-suprnova-upload-test'; connect-src 'self'",
    },
    html: uploadsScenario(),
  },
  lifecycleDestination: {
    html: document(
      `<h1>Lifecycle destination</h1><a id="lifecycle-return" href="/scenario/lifecycle">Return</a>`,
    ),
  },
  navigation: { html: navigationSource() },
  navigationDestination: { html: navigationDestination() },
  navigationPrefetch: {
    html: navigationDestination("Prefetch destination"),
    headers: { "cache-control": "public, max-age=60" },
  },
  navigationPrivate: {
    html: navigationDestination("Private destination"),
    headers: { "cache-control": "private, max-age=60" },
  },
  navigationHidden: { html: navigationDestination("Hidden destination") },
  navigationError: { html: navigationDestination("Not found"), status: 404 },
  documentTransition: { html: documentTransitionSource() },
  documentTransitionUnsupported: { html: documentTransitionSource({ unsupported: true }) },
  documentTransitionCaptureFailure: { html: documentTransitionSource({ captureFailure: true }) },
  documentTransitionDestination: {
    html: document(
      `<style>@view-transition { navigation: auto; }</style>
        <h1 id="transition-destination-focus" tabindex="-1" data-suprnova-live-document-focus>Document transition destination</h1>
        <div id="transition-hero" data-suprnova-live-document-transition="hero">Hero destination</div>
        <a href="/scenario/documentTransition">Return</a>`,
      navigationBoot(),
    ),
  },
  manual: { html: document(island()) },
  instance: { html: document(island(), moduleBoot()) },
  seed: {
    html: document(
      island({ envelope: seedEnvelope, form: "seed", instanceId: "", revision: "0" }),
      moduleBoot(),
    ),
  },
  malformed: {
    html: document(
      '<section data-suprnova-live-island data-suprnova-live-component="catalog.search"><p>Malformed but visible</p></section>',
      moduleBoot(),
    ),
  },
  duplicate: {
    html: document(
      `${island({ content: "First copy" })}${island({ content: "Second copy" })}`,
      moduleBoot(),
    ),
  },
  incompatible: {
    html: document(
      island({ protocolMinimum: "3", content: "Incompatible but visible" }),
      moduleBoot(),
    ),
  },
  snapshotMismatch: {
    html: document(
      island({ slot: "forged-slot", content: "Mismatched but visible" }),
      moduleBoot(),
    ),
  },
  nested: {
    html: document(
      island({
        body: `<p>Parent island</p>${island({
          documentKey: "child",
          envelope: {
            ...instanceEnvelope,
            body: {
              ...instanceEnvelope.body,
              instance_id: "EBESExQVFhcYGRobHB0eHw",
              slot: "child-slot",
            },
          },
          instanceId: "EBESExQVFhcYGRobHB0eHw",
          slot: "child-slot",
          content: "Child island",
        })}`,
      }),
      moduleBoot(),
    ),
  },
  duplicateRuntime: {
    html: document(
      island(),
      '<script src="/assets/suprnova-live.classic.js"></script><script>window.SuprnovaLive.boot();</script>' +
        moduleBoot(),
    ),
  },
  dynamic: {
    html: document(
      `<template id="candidate">${island({
        body: '<button id="dynamic-retired-action" type="button" live:click.prevent="save">Detached action</button>',
        documentKey: "dynamic",
      })}</template>`,
      moduleBoot(),
    ),
  },
  directives: {
    html: document(
      island({
        body: `<a id="native-action" href="#native" live:click="save">Native action</a>
          <button id="once-action" live:click.prevent.stop.once="save">Once action</button>
          <a id="trusted-action" href="#trusted" live:click.prevent.trusted="save">Trusted action</a>
          <button id="disabled-action" disabled live:click.prevent="save">Disabled action</button>
          <input id="key-action" live:keydown.enter.prevent="save">
          <button id="late-action">Late action</button>
          <button id="remove-action" live:click.prevent="save">Remove action</button>`,
      }),
      moduleBoot(),
    ),
  },
  directiveOwnership: {
    html: document(
      island({
        rootAttributes: ' live:click.prevent.self="parent"',
        body: `${island({
          documentKey: "child",
          envelope: {
            ...instanceEnvelope,
            body: {
              ...instanceEnvelope.body,
              instance_id: "EBESExQVFhcYGRobHB0eHw",
              slot: "child-slot",
            },
          },
          instanceId: "EBESExQVFhcYGRobHB0eHw",
          slot: "child-slot",
          body: '<button id="child-plain">Child plain</button><button id="child-owned" live:click.prevent="child">Child owned</button>',
        })}<open-live-host id="open-host"></open-live-host><closed-live-host id="closed-host"></closed-live-host>`,
      }),
      `<script>
        customElements.define("open-live-host", class extends HTMLElement {
          connectedCallback() {
            const root = this.attachShadow({ mode: "open" });
            root.innerHTML = '<button id="open-action" live:click.prevent="open">Open action</button>';
          }
        });
        customElements.define("closed-live-host", class extends HTMLElement {
          connectedCallback() {
            const root = this.attachShadow({ mode: "closed" });
            root.innerHTML = '<button id="closed-action" live:click.prevent="closed">Closed action</button>';
            window.__suprnovaClosedButton = root.querySelector("button");
          }
        });
      </script>${moduleBoot()}`,
    ),
  },
  seedAction: {
    html: document(
      island({
        body: '<button id="seed-action" live:click.prevent="save">Seed action</button>',
        envelope: seedEnvelope,
        form: "seed",
        instanceId: "",
        revision: "0",
      }),
      moduleBoot(),
    ),
  },
  networkInstance: {
    html: document(
      island({
        body: '<button id="network-action" live:click.prevent="search">Search</button>',
      }),
      moduleBoot(),
    ),
  },
  networkRetry: {
    html: document(
      island({
        body: '<button id="network-action" live:click.prevent="search">Search</button>',
      }),
      moduleBoot(),
      { endpoint: "/live?mode=retry" },
    ),
  },
  responseRedirect: {
    html: document(
      island({
        body: '<button id="response-action" live:click.prevent="search">Redirect</button><p id="response-content">Original</p>',
      }),
      responseOrderBoot(),
      { endpoint: "/live?mode=redirect" },
    ),
  },
  responseNavigated: {
    html: document(
      island({
        body: '<button id="response-action" live:click.prevent="search">Navigate</button><p id="response-content">Original</p>',
        protocolMinimum: "2",
      }),
      responseOrderBoot(),
      { endpoint: "/live?mode=navigated" },
    ),
  },
  responseCommitted: {
    html: document(
      island({
        body: '<button id="response-action" live:click.prevent="search">Update</button><p id="response-content">Original</p>',
        protocolMinimum: "2",
      }),
      responseOrderBoot(),
      { endpoint: "/live?mode=committed" },
    ),
  },
  responseNoRender: {
    html: document(
      island({
        body: '<button id="response-action" live:click.prevent="search">Update</button><p id="response-content">Original</p>',
        protocolMinimum: "2",
      }),
      responseOrderBoot(),
      { endpoint: "/live?mode=no-render" },
    ),
  },
  transitions: {
    html: transitionScenario(),
  },
  transitionsUnsupported: {
    html: transitionScenario(true),
  },
  recoveryFails: {
    html: document(
      island({
        body: '<button id="recovery-action" live:click.prevent="save">Recover</button><p id="recovery-content">Last accepted</p>',
        protocolMinimum: "2",
      }),
      moduleBoot(),
      { endpoint: "/live?mode=recovery-fails" },
    ),
  },
  feedback: {
    html: document(
      island({
        body: `<label>Name <input id="feedback-model" value="Ada" live:model.action="name"></label>
          <span id="feedback-dirty" hidden live:dirty.show="name">Unsaved changes</span>
          <button id="feedback-action" live:click.prevent="save" live:loading.disabled="save">Save</button>
          <div id="feedback-busy" live:loading.busy="save">Save status</div>
          <div id="feedback-retrying" hidden live:retrying.show="save">Trying again</div>
          <output id="feedback-live" live:retrying.live.polite="save"></output>
          <div id="feedback-combined" hidden live:idle.show="save" live:queued.show="save" live:loading.class="save">Combined status</div>
          <a id="feedback-escape" href="#feedback-escaped" live:loading.disabled="save">Cancel</a>`,
      }),
      moduleBoot(),
      { endpoint: "/live?mode=retry" },
    ),
  },
  networkSeed: {
    html: document(
      island({
        body: '<button id="network-action" live:click.prevent="search">Search</button>',
        envelope: seedEnvelope,
        form: "seed",
        instanceId: "",
        revision: "0",
      }),
      moduleBoot(),
    ),
  },
  seedActionNoCrypto: {
    html: document(
      island({
        body: '<button id="seed-action" live:click.prevent="save">Seed action</button>',
        envelope: seedEnvelope,
        form: "seed",
        instanceId: "",
        revision: "0",
      }),
      '<script type="module">import { boot } from "/assets/suprnova-live.esm.js"; boot({ randomness: { randomBytes() { throw new Error("crypto_unavailable"); } } });</script>',
    ),
  },
  lazySeed: {
    html: document(
      island({
        envelope: seedEnvelope,
        form: "seed",
        instanceId: "",
        revision: "0",
        rootAttributes: ' live:lazy.visible=""',
      }),
      moduleBoot(),
    ),
  },
  localSignals: {
    html: document(
      island({
        rootAttributes: ' live:signal="open:false,label:hello,count:1,none:null"',
        body: `<button id="signal-toggle" live:toggle="open">Toggle</button>
          <div id="signal-panel" role="region" hidden aria-hidden="true" inert live:show="open" live:class="is-open:open" live:attr="aria-expanded:open">Panel</div>
          <div id="signal-mismatch" live:show="open">Initially mismatched panel</div>
          <div role="tablist"><button id="signal-tab" role="tab" aria-selected="false" live:selected="open">Tab</button></div>
          <button id="signal-disclosure" aria-expanded="false" live:expanded="open">Disclosure</button>
          <div id="signal-guard" live:inert="open">Guard</div>
          <div id="signal-combined" hidden aria-hidden="true" inert live:show="open" live:inert="open">Combined</div>
          <input id="signal-focus" aria-label="Signal focus target" live:focus="open">
          <div data-suprnova-live-key="child" live:signal="open:true">
            <button id="child-toggle" live:toggle="open">Child toggle</button>
            <div id="child-panel" live:show="open">Child panel</div>
          </div>
          <div id="late-local">Late local</div>
          <div id="unsafe-local" live:attr="onclick:open"></div>`,
      }),
      moduleBoot(),
    ),
  },
  multipleSchedulers: {
    html: document(
      `${island({
        documentKey: "first-scheduler",
        rootAttributes: ' id="first-island"',
        body: '<a id="first-scheduler" href="#first-fallback" live:click.prevent="save">First scheduler</a>',
      })}${island({
        documentKey: "second-scheduler",
        envelope: {
          ...instanceEnvelope,
          body: {
            ...instanceEnvelope.body,
            instance_id: "ICEiIyQlJicoKSorLC0uLw",
            slot: "second-scheduler-slot",
          },
        },
        instanceId: "ICEiIyQlJicoKSorLC0uLw",
        rootAttributes: ' id="second-island"',
        slot: "second-scheduler-slot",
        body: '<a id="second-scheduler" href="#second-fallback" live:click.prevent="save">Second scheduler</a>',
      })}`,
      moduleBoot(),
    ),
  },
  modelsImmediate: {
    html: document(
      island({
        body: `<label>Query <input id="immediate-model" live:model.immediate="query"></label>
          <a id="immediate-after" href="#immediate-fallback" live:click.prevent="save">Save</a>`,
      }),
      moduleBoot(),
      { max_queued_per_island: 1 },
    ),
  },
  modelsDebounce: {
    html: document(
      island({
        body: `<label>Query <input id="debounced-model" live:model.debounce.100ms="query"></label>
          <a id="debounced-after" href="#debounced-fallback" live:click.prevent="save">Save</a>`,
      }),
      moduleBoot(),
      { max_queued_per_island: 1 },
    ),
  },
  modelsForm: {
    html: document(
      island({
        body: `<form id="model-form" action="/models-native" live:submit.prevent="save">
            <input id="model-query" name="query" value="initial" live:model.action="query">
            <input id="model-number" name="quantity" type="number" value="2" live:model.submit="quantity">
            <input id="model-checkbox" name="enabled" type="checkbox" live:model.submit="enabled">
            <select id="model-tags" name="tags" multiple live:model.submit="tags">
              <option value="rust" selected>Rust</option><option value="zig">Zig</option>
            </select>
            <input id="model-disabled" disabled value="ignored" live:model.immediate="disabled">
            <input id="model-file" name="attachment" type="file" live:model.immediate="attachment">
            <button id="model-reset" type="reset">Reset</button>
            <button id="model-submit" type="submit">Save</button>
          </form>`,
      }),
      moduleBoot(),
      { max_queued_per_island: 1 },
    ),
  },
  modelsNested: {
    html: document(
      island({
        body: `${island({
          documentKey: "model-child",
          envelope: {
            ...instanceEnvelope,
            body: {
              ...instanceEnvelope.body,
              instance_id: "EBESExQVFhcYGRobHB0eHw",
              slot: "model-child-slot",
            },
          },
          instanceId: "EBESExQVFhcYGRobHB0eHw",
          slot: "model-child-slot",
          body: '<input id="child-model" live:model.immediate="query">',
        })}<a id="parent-after-child" href="#parent-fallback" live:click.prevent="save">Parent save</a>`,
      }),
      moduleBoot(),
      { max_queued_per_island: 1 },
    ),
  },
  effects: {
    html: document(
      island({
        rootAttributes: ' live:signal="ready:false" live:effect="announce"',
        body: `<button id="extension-call" live:call="mark-ready">Mark ready</button>
          <div id="extension-panel" hidden aria-hidden="true" inert live:show="ready">Ready</div>
          <output id="effect-output"></output>`,
      }),
      extensionBoot(),
    ),
  },
  stimulus: {
    html: document(
      island({
        protocolMinimum: "2",
        rootAttributes: ' id="stimulus-island"',
        body: `<button id="stimulus-action" live:click.prevent="save">Morph</button>
          <div id="stimulus-preserved" data-controller="probe" data-probe="preserved" data-suprnova-live-key="preserved"></div>
            <div id="stimulus-removed" data-controller="probe" data-probe="removed" data-suprnova-live-key="removed"></div>
            <div id="stimulus-detached" data-controller="probe" data-probe="detached" data-suprnova-live-key="detached"></div>
            <div id="stimulus-throws" data-controller="probe" data-probe="throws" data-probe-throw data-suprnova-live-key="throws"></div>
            ${stimulusChild()}`,
      }),
      stimulusBoot(),
      { endpoint: "/live?mode=stimulus-morph" },
    ),
  },
  morphIdentity: {
    html: document(
      island({
        protocolMinimum: "2",
        body: `<button id="morph-action" live:click.prevent="save">Morph</button>
          <ol id="morph-list">
            <li id="alpha" data-suprnova-live-key="alpha">Alpha</li>
            <li id="beta" data-suprnova-live-key="beta">Beta</li>
            <li id="old" data-suprnova-live-key="old">Old</li>
          </ol>
          ${morphChild()}`,
      }),
      moduleBoot(),
      { endpoint: "/live?mode=morph-identity" },
    ),
  },
  morphUnsafe: {
    html: document(
      island({
        protocolMinimum: "2",
        body: `<button id="morph-unsafe-action" live:click.prevent="save">Morph</button>
          <p id="morph-unsafe-content">Original</p>`,
      }),
      morphFailureBoot(),
      { endpoint: "/live?mode=morph-unsafe" },
    ),
  },
  preservation: {
    html: document(
      `${island({
        protocolMinimum: "2",
        body: preservationBody(),
      })}<div id="modal-root" aria-label="Modal destination"></div>`,
      preservationBoot(),
      { endpoint: "/live?mode=preservation" },
    ),
  },
  continuity: {
    html: document(
      island({
        protocolMinimum: "2",
        body: continuityBody(),
      }),
      continuityBoot(),
      { endpoint: "/live?mode=continuity" },
    ),
  },
  teleportLateTarget: {
    html: document(
      island({
        protocolMinimum: "2",
        body: '<button id="late-teleport-action" live:click.prevent="save">Attempt teleport</button><p id="late-teleport-content">Original</p>',
      }),
      morphFailureBoot(),
      { endpoint: "/live?mode=teleport-late-target" },
    ),
  },
  cspNonce: {
    headers: {
      "content-security-policy":
        "default-src 'none'; script-src 'self' 'nonce-suprnova-test'; connect-src 'self'",
    },
    html: document(island(), moduleBoot(' nonce="suprnova-test"')),
  },
  cspHash: {
    headers: { "content-security-policy": hashPolicy() },
    html: document(island(), moduleBoot()),
  },
  cspBlocked: {
    headers: { "content-security-policy": "default-src 'none'; script-src 'none'" },
    html: document(island(), moduleBoot()),
  },
  cspModuleNonce: {
    headers: {
      "content-security-policy":
        "default-src 'none'; script-src 'nonce-suprnova-test' 'strict-dynamic'; connect-src 'self'",
    },
    html: document(island(), externalModuleScript("nonce")),
  },
  cspModuleHash: {
    headers: hashOnlyModulePolicy,
    html: document(island(), externalModuleScript("integrity")),
  },
  cspClassicNonce: {
    headers: {
      "content-security-policy":
        "default-src 'none'; script-src 'nonce-suprnova-test' 'strict-dynamic'; connect-src 'self'",
    },
    html: document(island(), externalClassicScripts("nonce")),
  },
  cspClassicHash: {
    headers: hashOnlyClassicPolicy,
    html: hashOnlyClassicDocument,
  },
});
