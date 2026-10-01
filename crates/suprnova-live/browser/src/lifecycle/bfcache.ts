import { ISLAND_ROOT_SELECTOR, parseIslandMetadata } from "../islands/metadata.js";
import { parseRuntimeConfig } from "../runtime/config.js";
import type { BootstrapOptions, RuntimeConfig } from "../runtime/types.js";
import type { DocumentLifecycleCompatibility } from "./document.js";

export type RestoreIncompatibility = "asset" | "island" | "protocol" | "runtime";

export interface RestoreCompatibilityResult {
  readonly compatible: boolean;
  readonly reason: RestoreIncompatibility | null;
}

const MAX_RESTORE_ISLANDS = 256;

function incompatible(reason: RestoreIncompatibility): RestoreCompatibilityResult {
  return Object.freeze({ compatible: false, reason });
}

export function validateDocumentRestore(
  expected: RuntimeConfig,
  current: RuntimeConfig,
  document: Document,
): RestoreCompatibilityResult {
  const currentRuntimeContract: number = current.runtimeContractVersion;
  const expectedRuntimeContract: number = expected.runtimeContractVersion;
  if (currentRuntimeContract !== expectedRuntimeContract) {
    return incompatible("runtime");
  }
  if (current.assetIdentity !== expected.assetIdentity) return incompatible("asset");
  if (
    current.protocol.minimum !== expected.protocol.minimum ||
    current.protocol.maximum !== expected.protocol.maximum
  ) {
    return incompatible("protocol");
  }
  const islands = document.querySelectorAll(ISLAND_ROOT_SELECTOR);
  if (islands.length > MAX_RESTORE_ISLANDS) return incompatible("island");
  try {
    for (const island of islands) parseIslandMetadata(island, current);
  } catch {
    return incompatible("island");
  }
  return Object.freeze({ compatible: true, reason: null });
}

export class BrowserRestoreCompatibility implements DocumentLifecycleCompatibility {
  readonly #document: Document;
  readonly #expected: RuntimeConfig;
  readonly #options: BootstrapOptions;
  #last: RestoreCompatibilityResult = Object.freeze({ compatible: true, reason: null });

  constructor(document: Document, expected: RuntimeConfig, options: BootstrapOptions = {}) {
    this.#document = document;
    this.#expected = expected;
    this.#options = options;
  }

  validate(): boolean {
    let current: RuntimeConfig;
    try {
      current = parseRuntimeConfig(this.#document, this.#options);
    } catch {
      this.#last = incompatible("runtime");
      return false;
    }
    this.#last = validateDocumentRestore(this.#expected, current, this.#document);
    return this.#last.compatible;
  }

  result(): RestoreCompatibilityResult {
    return this.#last;
  }
}
