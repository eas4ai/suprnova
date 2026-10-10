import type { OwnedDirective } from "../directives/ownership.js";
import type { RuntimeDiagnosticSink } from "../runtime/diagnostics.js";
import type { SignalTarget } from "./graph.js";
import type { LocalSignalScope } from "./scope.js";
import type { SignalValue } from "./value.js";

const SAFE_CLASS = /^[A-Za-z][A-Za-z0-9_-]{0,127}$/u;
const SAFE_ATTRIBUTE = /^[A-Za-z][A-Za-z0-9_-]{0,127}$/u;
const FORBIDDEN_ATTRIBUTES = new Set([
  "action",
  "background",
  "cite",
  "crossorigin",
  "data",
  "formaction",
  "formenctype",
  "formmethod",
  "formtarget",
  "href",
  "integrity",
  "is",
  "manifest",
  "method",
  "nonce",
  "ping",
  "poster",
  "profile",
  "referrerpolicy",
  "rel",
  "src",
  "srcdoc",
  "srcset",
  "style",
  "target",
  "type",
  "usemap",
  "xlink-href",
]);

interface BooleanPresentationSource {
  current(): boolean;
}

interface ElementPresentationState {
  show?: BooleanPresentationSource;
  inert?: BooleanPresentationSource;
}

const ELEMENT_PRESENTATION = new WeakMap<Element, ElementPresentationState>();

function elementPresentation(element: Element): ElementPresentationState {
  const existing = ELEMENT_PRESENTATION.get(element);
  if (existing !== undefined) return existing;
  const created = {};
  ELEMENT_PRESENTATION.set(element, created);
  return created;
}

function reconcileVisibility(element: Element, state: ElementPresentationState): void {
  const visible = state.show?.current() ?? !element.hasAttribute("hidden");
  element.toggleAttribute("hidden", !visible);
  element.setAttribute("aria-hidden", String(!visible));
  element.toggleAttribute("inert", !visible || (state.inert?.current() ?? false));
}

export type AttributeProjection =
  Readonly<{ kind: "remove" }> | Readonly<{ kind: "set"; value: string }>;

export function isSafeClassName(name: string): boolean {
  return SAFE_CLASS.test(name);
}

export function isSafeAttributeName(name: string): boolean {
  const normalized = name.toLowerCase();
  const moduleDataAttribute =
    normalized === "data-action" ||
    normalized === "data-controller" ||
    /^data-[a-z0-9_-]+-(?:class|outlet|target|value)$/u.test(normalized);
  return (
    SAFE_ATTRIBUTE.test(name) &&
    !normalized.startsWith("on") &&
    !normalized.startsWith("data-suprnova-live-") &&
    !moduleDataAttribute &&
    !FORBIDDEN_ATTRIBUTES.has(normalized)
  );
}

export function presentationBoolean(value: SignalValue): boolean {
  if (typeof value !== "boolean") throw new Error("presentation_boolean_required");
  return value;
}

export function attributeProjection(name: string, value: SignalValue): AttributeProjection {
  if (!isSafeAttributeName(name)) throw new Error("presentation_attribute_unsafe");
  if (value === null) return Object.freeze({ kind: "remove" });
  if (typeof value === "boolean") {
    if (name.toLowerCase().startsWith("aria-")) {
      return Object.freeze({ kind: "set", value: String(value) });
    }
    return value ? Object.freeze({ kind: "set", value: "" }) : Object.freeze({ kind: "remove" });
  }
  return Object.freeze({ kind: "set", value: String(value) });
}

function mapping(value: string): readonly Readonly<{ name: string; signal: string }>[] {
  return Object.freeze(
    value.split(",").map((entry) => {
      const separator = entry.indexOf(":");
      if (separator <= 0 || separator !== entry.lastIndexOf(":")) {
        throw new Error("presentation_mapping_invalid");
      }
      return Object.freeze({ name: entry.slice(0, separator), signal: entry.slice(separator + 1) });
    }),
  );
}

function mismatch(diagnostics: RuntimeDiagnosticSink): void {
  diagnostics.record({
    code: "directive_invalid",
    severity: "warning",
    phase: "directive",
    detailCode: "contract_mismatch",
  });
}

abstract class PresentationTarget implements SignalTarget {
  readonly element: Element;
  readonly scope: LocalSignalScope;
  readonly signal: string;
  readonly #diagnostics: RuntimeDiagnosticSink;
  #initial = true;

  constructor(
    element: Element,
    scope: LocalSignalScope,
    signal: string,
    diagnostics: RuntimeDiagnosticSink,
  ) {
    this.element = element;
    this.scope = scope;
    this.signal = signal;
    this.#diagnostics = diagnostics;
  }

  apply(): void {
    const value = this.scope.get(this.signal);
    if (this.#initial) {
      this.#initial = false;
      if (!this.matches(value)) mismatch(this.#diagnostics);
    }
    this.project(value);
  }

  dispose(): void {
    return undefined;
  }

  protected abstract matches(value: SignalValue): boolean;
  protected abstract project(value: SignalValue): void;
}

class ShowTarget extends PresentationTarget {
  readonly #state: ElementPresentationState;

  constructor(
    element: Element,
    scope: LocalSignalScope,
    signal: string,
    diagnostics: RuntimeDiagnosticSink,
  ) {
    super(element, scope, signal, diagnostics);
    this.#state = elementPresentation(element);
    this.#state.show = this;
  }

  protected matches(value: SignalValue): boolean {
    const visible = presentationBoolean(value);
    return (
      !this.element.hasAttribute("hidden") === visible &&
      this.element.getAttribute("aria-hidden") === String(!visible) &&
      this.element.hasAttribute("inert") === (!visible || (this.#state.inert?.current() ?? false))
    );
  }

  current(): boolean {
    return presentationBoolean(this.scope.get(this.signal));
  }

  override dispose(): void {
    if (this.#state.show === this) delete this.#state.show;
  }

  protected project(): void {
    reconcileVisibility(this.element, this.#state);
  }
}

class ClassTarget extends PresentationTarget {
  readonly #className: string;

  constructor(
    element: Element,
    scope: LocalSignalScope,
    signal: string,
    diagnostics: RuntimeDiagnosticSink,
    className: string,
  ) {
    if (!isSafeClassName(className)) throw new Error("presentation_class_unsafe");
    super(element, scope, signal, diagnostics);
    this.#className = className;
  }

  protected matches(value: SignalValue): boolean {
    return this.element.classList.contains(this.#className) === presentationBoolean(value);
  }

  protected project(value: SignalValue): void {
    this.element.classList.toggle(this.#className, presentationBoolean(value));
  }
}

class AttributeTarget extends PresentationTarget {
  readonly #name: string;

  constructor(
    element: Element,
    scope: LocalSignalScope,
    signal: string,
    diagnostics: RuntimeDiagnosticSink,
    name: string,
  ) {
    if (!isSafeAttributeName(name)) throw new Error("presentation_attribute_unsafe");
    super(element, scope, signal, diagnostics);
    this.#name = name;
  }

  protected matches(value: SignalValue): boolean {
    const projection = attributeProjection(this.#name, value);
    return projection.kind === "remove"
      ? !this.element.hasAttribute(this.#name)
      : this.element.getAttribute(this.#name) === projection.value;
  }

  protected project(value: SignalValue): void {
    const projection = attributeProjection(this.#name, value);
    if (projection.kind === "remove") this.element.removeAttribute(this.#name);
    else this.element.setAttribute(this.#name, projection.value);
  }
}

class BooleanAttributeTarget extends PresentationTarget {
  readonly #attribute: string;

  constructor(
    element: Element,
    scope: LocalSignalScope,
    signal: string,
    diagnostics: RuntimeDiagnosticSink,
    attribute: string,
  ) {
    super(element, scope, signal, diagnostics);
    this.#attribute = attribute;
  }

  protected matches(value: SignalValue): boolean {
    return this.element.getAttribute(this.#attribute) === String(presentationBoolean(value));
  }

  protected project(value: SignalValue): void {
    const selected = presentationBoolean(value);
    this.element.setAttribute(this.#attribute, String(selected));
    if (this.#attribute === "aria-selected" && this.element.tagName === "OPTION") {
      (this.element as HTMLOptionElement).selected = selected;
    }
  }
}

class InertTarget extends PresentationTarget {
  readonly #state: ElementPresentationState;

  constructor(
    element: Element,
    scope: LocalSignalScope,
    signal: string,
    diagnostics: RuntimeDiagnosticSink,
  ) {
    super(element, scope, signal, diagnostics);
    this.#state = elementPresentation(element);
    this.#state.inert = this;
  }

  protected matches(value: SignalValue): boolean {
    const explicitlyInert = presentationBoolean(value);
    return (
      this.element.hasAttribute("inert") ===
      (!(this.#state.show?.current() ?? true) || explicitlyInert)
    );
  }

  current(): boolean {
    return presentationBoolean(this.scope.get(this.signal));
  }

  override dispose(): void {
    if (this.#state.inert === this) delete this.#state.inert;
  }

  protected project(): void {
    reconcileVisibility(this.element, this.#state);
  }
}

class FocusTarget extends PresentationTarget {
  protected matches(value: SignalValue): boolean {
    const focused = this.element.ownerDocument.activeElement === this.element;
    return focused === presentationBoolean(value);
  }

  protected project(value: SignalValue): void {
    if (!presentationBoolean(value) || this.element.hasAttribute("inert")) return;
    const focusable = this.element as HTMLElement;
    if (typeof focusable.focus === "function") focusable.focus();
  }
}

export interface PresentationBinding {
  readonly signal: string;
  readonly target: SignalTarget;
}

export function buildPresentationBindings(
  owned: OwnedDirective,
  scope: LocalSignalScope,
  diagnostics: RuntimeDiagnosticSink,
): readonly PresentationBinding[] {
  const { directive, element } = owned;
  try {
    switch (directive.name) {
      case "show":
        return [
          {
            signal: directive.value,
            target: new ShowTarget(element, scope, directive.value, diagnostics),
          },
        ];
      case "class":
        return mapping(directive.value).map(({ name, signal }) => ({
          signal,
          target: new ClassTarget(element, scope, signal, diagnostics, name),
        }));
      case "attr":
        return mapping(directive.value).map(({ name, signal }) => ({
          signal,
          target: new AttributeTarget(element, scope, signal, diagnostics, name),
        }));
      case "selected":
        return [
          {
            signal: directive.value,
            target: new BooleanAttributeTarget(
              element,
              scope,
              directive.value,
              diagnostics,
              "aria-selected",
            ),
          },
        ];
      case "expanded":
        return [
          {
            signal: directive.value,
            target: new BooleanAttributeTarget(
              element,
              scope,
              directive.value,
              diagnostics,
              "aria-expanded",
            ),
          },
        ];
      case "inert":
        return [
          {
            signal: directive.value,
            target: new InertTarget(element, scope, directive.value, diagnostics),
          },
        ];
      case "focus":
        return [
          {
            signal: directive.value,
            target: new FocusTarget(element, scope, directive.value, diagnostics),
          },
        ];
      default:
        return [];
    }
  } catch {
    diagnostics.record({
      code: "directive_invalid",
      severity: "error",
      phase: "directive",
      detailCode: "operation_rejected",
    });
    return [];
  }
}
