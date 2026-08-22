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

function config() {
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
  })}</script>`;
}

function document(body, scripts = "") {
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Suprnova Live conformance</title></head><body>${config()}<main>${body}</main>${scripts}</body></html>`;
}

function moduleBoot(attributes = "") {
  return `<script type="module"${attributes}>${bootSource}</script>`;
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
