import type { RuntimeScheduler } from "../runtime/ports.js";
import type { LocalSignalScope } from "./scope.js";

export interface SignalTarget {
  readonly element: Element;
  apply(): void;
  dispose(): void;
}

interface RegisteredTarget {
  readonly owner: LocalSignalScope;
  readonly signal: string;
  readonly target: SignalTarget;
}

const MAX_SIGNAL_TARGETS = 4_096;
const MAX_SIGNAL_FLUSH_TARGETS = 4_096;

function documentOrder(left: SignalTarget, right: SignalTarget): number {
  if (left.element === right.element) return 0;
  const position = left.element.compareDocumentPosition(right.element);
  return position & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1;
}

export class SignalGraph {
  readonly #scheduler: RuntimeScheduler;
  readonly #byOwner = new Map<LocalSignalScope, Map<string, Set<RegisteredTarget>>>();
  readonly #registrations = new Set<RegisteredTarget>();
  readonly #pending = new Set<RegisteredTarget>();
  #scheduled = false;
  #disposed = false;

  constructor(scheduler: RuntimeScheduler) {
    this.#scheduler = scheduler;
  }

  register(scope: LocalSignalScope, signal: string, target: SignalTarget): VoidFunction {
    if (this.#disposed) throw new Error("signal_graph_disposed");
    if (this.#registrations.size >= MAX_SIGNAL_TARGETS) throw new Error("signal_target_limit");
    const owner = scope.owner(signal);
    const registration = { owner, signal, target };
    target.apply();
    let bySignal = this.#byOwner.get(owner);
    if (bySignal === undefined) {
      bySignal = new Map();
      this.#byOwner.set(owner, bySignal);
    }
    let targets = bySignal.get(signal);
    if (targets === undefined) {
      targets = new Set();
      bySignal.set(signal, targets);
    }
    targets.add(registration);
    this.#registrations.add(registration);
    let removed = false;
    return () => {
      if (removed) return;
      removed = true;
      targets.delete(registration);
      this.#registrations.delete(registration);
      this.#pending.delete(registration);
      target.dispose();
    };
  }

  changed(scope: LocalSignalScope, names: readonly string[]): void {
    if (this.#disposed) return;
    const bySignal = this.#byOwner.get(scope);
    if (bySignal === undefined) return;
    for (const name of names) {
      for (const registration of bySignal.get(name) ?? []) {
        if (this.#pending.size >= MAX_SIGNAL_FLUSH_TARGETS) break;
        this.#pending.add(registration);
      }
    }
    if (this.#pending.size === 0 || this.#scheduled) return;
    this.#scheduled = true;
    this.#scheduler.microtask(() => {
      this.#flush();
    });
  }

  dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    for (const registration of this.#registrations) registration.target.dispose();
    this.#registrations.clear();
    this.#pending.clear();
    this.#byOwner.clear();
  }

  #flush(): void {
    this.#scheduled = false;
    if (this.#disposed) return;
    const pending = [...this.#pending].sort((left, right) =>
      documentOrder(left.target, right.target),
    );
    this.#pending.clear();
    for (const registration of pending) {
      if (this.#registrations.has(registration)) registration.target.apply();
    }
  }
}
