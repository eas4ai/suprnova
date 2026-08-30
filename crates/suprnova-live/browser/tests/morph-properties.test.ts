import fc from "fast-check";
import { describe, expect, it } from "vitest";

import { parseIslandMetadata, IslandMetadataError } from "../src/islands/metadata.js";
import { DocumentLifecycle } from "../src/lifecycle/document.js";
import { ResourceLedgerImpl } from "../src/lifecycle/resources.js";
import { preflightIslandMorph } from "../src/morph/preflight.js";
import type { RuntimeConfig } from "../src/runtime/types.js";
import {
  asElement,
  element,
  FakeDocument,
  type FakeElement,
  morphFixture,
} from "./support/morph-dom.js";

const PROPERTY_SEED = 0x31415926;
const METADATA_DETAILS = [
  "attribute",
  "identity",
  "lazy_complete",
  "metadata_limit",
  "protocol",
  "revision",
  "root_marker",
  "root_slot",
  "runtime_contract",
  "snapshot",
  "snapshot_disagreement",
  "snapshot_form",
] as const;
const CONFIG: RuntimeConfig = Object.freeze({
  runtimeContractVersion: 1,
  protocol: Object.freeze({ minimum: 1, maximum: 2 }),
  endpoint: new URL("https://app.example.test/_suprnova/live"),
  credentials: "same-origin",
  requestTimeoutMs: 15_000,
  maxResponseBytes: 1_048_576,
  maxQueuedPerIsland: 16,
  maxParallelPerIsland: 1,
  assetIdentity: "property-runtime-v1",
});
const BASE_METADATA = Object.freeze({
  "data-suprnova-live-component": "catalog.search",
  "data-suprnova-live-contract": "1",
  "data-suprnova-live-document-key": "property-root",
  "data-suprnova-live-island": "",
  "data-suprnova-live-lazy-complete": "false",
  "data-suprnova-live-protocol-min": "1",
  "data-suprnova-live-revision": "0",
  "data-suprnova-live-root": "primary",
  "data-suprnova-live-slot": "primary",
  "data-suprnova-live-snapshot": "not-a-snapshot",
  "data-suprnova-live-snapshot-kind": "seed",
});

function forest(
  document: FakeDocument,
  keys: readonly string[],
  parents: readonly number[],
): readonly FakeElement[] {
  const nodes = keys.map((key) => element(document, "div", { "data-suprnova-live-key": key }));
  const roots: FakeElement[] = [];
  for (const [index, node] of nodes.entries()) {
    const parent = index === 0 ? index : (parents[index] ?? index) % (index + 1);
    if (parent === index) roots.push(node);
    else nodes[parent]?.append(node);
  }
  return roots;
}

function pageEvent(type: "pagehide" | "pageshow", persisted: boolean): Event {
  const event = new Event(type);
  Object.defineProperty(event, "persisted", { value: persisted });
  return event;
}

describe("morph and metadata properties", () => {
  it("plans bounded unique-key forests under arbitrary legal reparenting", () => {
    const keys = fc.uniqueArray(fc.integer({ min: 0, max: 1_000_000 }), {
      maxLength: 40,
      minLength: 1,
    });
    fc.assert(
      fc.property(keys, fc.array(fc.nat(64), { maxLength: 40 }), (identities, parents) => {
        const names = identities.map((identity) => `key-${String(identity)}`);
        const currentDocument = new FakeDocument();
        const replacementDocument = new FakeDocument();
        const current = forest(currentDocument, names, parents);
        const replacement = forest(
          replacementDocument,
          [...names].reverse(),
          [...parents].reverse(),
        );
        const fixture = morphFixture({
          currentChildren: current,
          replacementChildren: replacement,
        });
        const plan = preflightIslandMorph({
          authority: fixture.authority,
          currentRoot: asElement(fixture.currentRoot),
          html: "<section></section>",
          limits: fixture.limits,
          parser: fixture.parser,
        });
        expect(plan.identity.entries.length).toBe(names.length);
        expect(plan.identity.inserted).toEqual([]);
        expect(plan.identity.removed).toEqual([]);
        expect(new Set(plan.identity.moved).size).toBe(plan.identity.moved.length);
        expect(plan.identity.moved.length).toBeLessThanOrEqual(names.length);
      }),
      { numRuns: 250, seed: PROPERTY_SEED },
    );
  });

  it("classifies arbitrary DOM metadata bytes without raw echo", () => {
    const attribute = fc.constantFrom(...Object.keys(BASE_METADATA));
    const bytes = fc.uint8Array({ maxLength: 256 }).map((value) => String.fromCharCode(...value));
    fc.assert(
      fc.property(attribute, bytes, (name, value) => {
        const marker = `raw-secret:${value}:end`;
        const document = new FakeDocument();
        const root = element(document, "section", { ...BASE_METADATA, [name]: marker });
        try {
          const metadata = parseIslandMetadata(root as unknown as Element, CONFIG);
          expect(Object.isFrozen(metadata)).toBe(true);
        } catch (error: unknown) {
          expect(error).toBeInstanceOf(IslandMetadataError);
          if (!(error instanceof IslandMetadataError)) throw error;
          expect(["invalid", "incompatible"]).toContain(error.kind);
          expect(METADATA_DETAILS).toContain(error.detail);
          expect(error.message).toBe(`island_${error.kind}`);
          expect(JSON.stringify(error)).not.toContain(marker);
        }
      }),
      { numRuns: 400, seed: PROPERTY_SEED + 1 },
    );
  });

  it("rejects a root with an unbounded total attribute set before snapshot parsing", () => {
    const attributes: Record<string, string> = { ...BASE_METADATA };
    for (let index = 0; index < 257; index += 1) attributes[`aria-property-${String(index)}`] = "";
    const root = element(new FakeDocument(), "section", attributes);
    expect(() => parseIslandMetadata(root as unknown as Element, CONFIG)).toThrow(
      expect.objectContaining({ detail: "metadata_limit" }),
    );
  });
});

describe("document lifecycle trace properties", () => {
  it("keeps state, epoch, callbacks, and resources bounded for arbitrary event traces", () => {
    const command = fc.constantFrom(
      "start",
      "suspend",
      "restore",
      "dispose",
      "freeze",
      "resume",
      "pagehide-persisted",
      "pagehide-replaced",
      "pageshow-persisted",
    );
    fc.assert(
      fc.property(fc.array(command, { maxLength: 80 }), (commands) => {
        const window = new EventTarget();
        const document = new EventTarget();
        const ledger = new ResourceLedgerImpl();
        const lifecycle = new DocumentLifecycle({
          compatibility: { validate: () => true },
          document,
          ledger,
          supportsFreezeResume: true,
          window,
        });
        let guardedCalls = 0;
        for (const command of commands) {
          const guarded = lifecycle.guard(() => {
            guardedCalls += 1;
          });
          switch (command) {
            case "start":
              if (lifecycle.state() === "disposed") {
                expect(() => {
                  lifecycle.start();
                }).toThrow("document_lifecycle_disposed");
              } else {
                lifecycle.start();
              }
              break;
            case "suspend":
              lifecycle.suspend();
              break;
            case "restore":
              lifecycle.restore();
              break;
            case "dispose":
              lifecycle.dispose();
              break;
            case "freeze":
            case "resume":
              document.dispatchEvent(new Event(command));
              break;
            case "pagehide-persisted":
              window.dispatchEvent(pageEvent("pagehide", true));
              break;
            case "pagehide-replaced":
              window.dispatchEvent(pageEvent("pagehide", false));
              break;
            case "pageshow-persisted":
              window.dispatchEvent(pageEvent("pageshow", true));
              break;
          }
          guarded();
          expect(["created", "active", "suspended", "restoring", "disposed"]).toContain(
            lifecycle.state(),
          );
          expect(lifecycle.epoch()).toBeLessThanOrEqual(commands.length);
          expect(Object.values(ledger.counts()).every((count) => count >= 0 && count <= 4)).toBe(
            true,
          );
        }
        expect(guardedCalls).toBeLessThanOrEqual(commands.length);
      }),
      { numRuns: 250, seed: PROPERTY_SEED + 2 },
    );
  });
});
