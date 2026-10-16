import type { RuntimeDiagnosticSink } from "../runtime/diagnostics.js";
import { captureStimulusContinuity } from "./lifecycle.js";
import type {
  StimulusApplicationPort,
  StimulusBootstrapOptions,
  StimulusContinuity,
  StimulusMorphBridge,
} from "./port.js";

const MAX_STIMULUS_DEFINITIONS = 256;
const MAX_ACTIVE_CONTINUITIES = 64;
const SAFE_IDENTIFIER = /^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$/u;

interface ContinuityState {
  readonly scope: Element;
  active: boolean;
}

function property(value: unknown, name: string): unknown {
  if (typeof value !== "object" || value === null) return undefined;
  try {
    return Reflect.get(value, name);
  } catch {
    return undefined;
  }
}

function applicationPort(value: unknown): value is StimulusApplicationPort {
  return (
    typeof value === "object" &&
    value !== null &&
    ["load", "start", "stop", "unload"].every((name) => typeof property(value, name) === "function")
  );
}

function definitionIdentifiers(definitions: readonly unknown[]): readonly string[] | null {
  const identifiers: string[] = [];
  const seen = new Set<string>();
  for (const definition of definitions) {
    if (typeof definition !== "object" || definition === null) return null;
    const identifier = property(definition, "identifier");
    if (
      typeof identifier !== "string" ||
      !SAFE_IDENTIFIER.test(identifier) ||
      seen.has(identifier)
    ) {
      return null;
    }
    seen.add(identifier);
    identifiers.push(identifier);
  }
  return Object.freeze(identifiers);
}

function contains(scope: Element, candidate: Element): boolean {
  try {
    return scope === candidate || scope.contains(candidate);
  } catch {
    return scope === candidate;
  }
}

class ApplicationStimulusBridge implements StimulusMorphBridge {
  readonly #application: StimulusApplicationPort | null;
  readonly #diagnostics: RuntimeDiagnosticSink;
  readonly #continuities = new Map<StimulusContinuity, ContinuityState>();
  #identifiers: readonly string[] = Object.freeze([]);
  #disposed = false;

  constructor(options: StimulusBootstrapOptions, diagnostics: RuntimeDiagnosticSink) {
    this.#diagnostics = diagnostics;
    const application = property(options, "application");
    if (!applicationPort(application)) {
      this.#application = null;
      this.#failure();
      return;
    }
    this.#application = application;
    const definitions = property(options, "definitions");
    if (
      definitions !== undefined &&
      (!Array.isArray(definitions) || definitions.length > MAX_STIMULUS_DEFINITIONS)
    ) {
      this.#failure("resource_exhausted");
    } else if (Array.isArray(definitions) && definitions.length > 0) {
      const identifiers = definitionIdentifiers(definitions);
      if (identifiers === null) {
        this.#failure();
        try {
          application.start();
        } catch {
          this.#failure();
        }
        return;
      }
      this.#identifiers = identifiers;
      try {
        const normalizedDefinitions: readonly unknown[] = definitions;
        application.load(...normalizedDefinitions);
      } catch {
        this.#failure();
      }
    }
    try {
      application.start();
    } catch {
      this.#failure();
    }
  }

  beforeMorph(scope: Element): StimulusContinuity {
    if (this.#disposed) {
      this.#failure();
      return Object.freeze({ roots: Object.freeze([]), scope, scopeIdentity: null });
    }
    if (this.#continuities.size >= MAX_ACTIVE_CONTINUITIES) {
      this.#failure("resource_exhausted");
      return Object.freeze({ roots: Object.freeze([]), scope, scopeIdentity: null });
    }
    let continuity: StimulusContinuity;
    try {
      continuity = captureStimulusContinuity(scope);
    } catch {
      this.#failure();
      continuity = Object.freeze({ roots: Object.freeze([]), scope, scopeIdentity: null });
    }
    this.#continuities.set(continuity, { active: true, scope });
    return continuity;
  }

  afterMorph(continuity: StimulusContinuity, scope: Element): void {
    const state = this.#continuities.get(continuity);
    if (state === undefined || !state.active || this.#disposed) {
      this.#failure();
      return;
    }
    state.active = false;
    this.#continuities.delete(continuity);
    try {
      const current = captureStimulusContinuity(scope);
      if (continuity.scopeIdentity !== null && current.scopeIdentity !== continuity.scopeIdentity) {
        throw new Error("stimulus_scope_identity_changed");
      }
    } catch {
      this.#failure();
    }
  }

  disposeScope(scope: Element): void {
    for (const [continuity, state] of this.#continuities) {
      if (!contains(scope, state.scope)) continue;
      state.active = false;
      this.#continuities.delete(continuity);
    }
  }

  dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    for (const state of this.#continuities.values()) state.active = false;
    this.#continuities.clear();
    if (this.#application === null) return;
    if (this.#identifiers.length > 0) {
      try {
        this.#application.unload(...this.#identifiers);
      } catch {
        this.#failure();
      }
      this.#identifiers = Object.freeze([]);
    }
    try {
      this.#application.stop();
    } catch {
      this.#failure();
    }
  }

  #failure(detailCode: "operation_rejected" | "resource_exhausted" = "operation_rejected") {
    this.#diagnostics.record({
      code: "lifecycle_notice",
      severity: "error",
      phase: "lifecycle",
      detailCode,
    });
  }
}

export function createStimulusMorphBridge(
  options: StimulusBootstrapOptions,
  diagnostics: RuntimeDiagnosticSink,
): StimulusMorphBridge {
  return new ApplicationStimulusBridge(options, diagnostics);
}

export type { StimulusApplicationPort } from "./port.js";
