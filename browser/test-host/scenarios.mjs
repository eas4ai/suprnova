import { createHash } from "node:crypto";

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
  return `<script type="module"${attributes}>${bootSource}</script>`;
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

function stimulusBoot() {
  return `<script type="module">
    import { Application, Controller } from "/test-vendor/stimulus.js";
    import { boot } from "/assets/suprnova-live.esm.js";

    const counts = new Map();
    const counter = (name) => {
      const current = counts.get(name) ?? { connect: 0, disconnect: 0 };
      counts.set(name, current);
      return current;
    };
    class ProbeController extends Controller {
      connect() {
        counter(this.element.dataset.probe).connect += 1;
        if (this.element.hasAttribute("data-probe-throw")) throw new Error("test controller failure");
      }
      disconnect() {
        counter(this.element.dataset.probe).disconnect += 1;
      }
    }

    let errors = 0;
    const application = new Application(document.documentElement);
    application.handleError = () => { errors += 1; };
    const runtime = boot({
      stimulus: {
        application,
        definitions: [{ identifier: "probe", controllerConstructor: ProbeController }],
      },
    });
      const until = async (predicate) => {
        for (let turn = 0; turn < 100; turn += 1) {
          if (predicate()) return;
          await new Promise((resolve) => setTimeout(resolve, 10));
        }
      throw new Error("stimulus lifecycle did not settle");
    };
      if (document.readyState === "loading") {
      await new Promise((resolve) => document.addEventListener("DOMContentLoaded", resolve, { once: true }));
      }
      await until(() => counter("preserved").connect === 1 && errors === 1);

      document.querySelector("#stimulus-action")?.click();
      await until(
        () =>
          document.querySelector("#stimulus-preserved")?.getAttribute("data-state") === "morphed" &&
          counter("removed").disconnect === 1 &&
          counter("inserted").connect === 1,
      );
      if (counter("preserved").connect !== 1 || counter("preserved").disconnect !== 0) {
        throw new Error("preserved controller duplicated");
      }

    document.documentElement.dataset.stimulusRuntimeAfterError =
      document.querySelector("#stimulus-island")?.getAttribute("data-suprnova-live-status") ?? "missing";
    runtime.stop();
    let disposal = "complete";
    try {
      await until(
          () =>
            counter("preserved").disconnect === 1 &&
            counter("inserted").disconnect === 1 &&
            counter("detached").disconnect === 1 &&
            counter("nested").disconnect === 1,
      );
    } catch {
      disposal = "incomplete";
    }
    const expose = (name) => {
      const value = counter(name);
      document.documentElement.dataset['stimulus' + name[0].toUpperCase() + name.slice(1)] =
        value.connect + ":" + value.disconnect;
    };
    for (const name of ["preserved", "removed", "inserted", "detached", "nested"]) expose(name);
    document.documentElement.dataset.stimulusDisposal = disposal;
    document.documentElement.dataset.stimulusErrors = String(errors);
    document.documentElement.dataset.stimulusReady = "true";
  </script>`;
}

function hashPolicy() {
  const digest = createHash("sha256").update(bootSource).digest("base64");
  return `default-src 'none'; script-src 'self' 'sha256-${digest}'; connect-src 'self'`;
}

export const scenarios = Object.freeze({
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
      `<template id="candidate">${island({ documentKey: "dynamic" })}</template>`,
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
});
