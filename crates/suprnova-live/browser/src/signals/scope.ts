import type { SignalDeclaration, SignalValue } from "./value.js";

export interface SignalScope {
  readonly identity: string;
  get(name: string): SignalValue;
  set(name: string, value: SignalValue): void;
  toggle(name: string): void;
  reset(name?: string): void;
  batch(update: () => void): void;
  dispose(): void;
}

export type SignalChangeCallback = (scope: LocalSignalScope, names: readonly string[]) => void;

interface SignalCell {
  readonly initial: SignalValue;
  value: SignalValue;
}

function compatible(initial: SignalValue, value: SignalValue): boolean {
  if (initial === null) return value === null;
  return (
    typeof initial === typeof value && (typeof value !== "number" || Number.isSafeInteger(value))
  );
}

export class LocalSignalScope implements SignalScope {
  readonly identity: string;
  readonly #parent: LocalSignalScope | null;
  readonly #cells = new Map<string, SignalCell>();
  readonly #onChange: SignalChangeCallback;
  readonly #pending = new Set<string>();
  #batchDepth = 0;
  #disposed = false;

  constructor(
    identity: string,
    declarations: readonly SignalDeclaration[],
    parent: LocalSignalScope | null = null,
    onChange: SignalChangeCallback = () => undefined,
  ) {
    if (identity.length === 0 || identity.length > 128) throw new Error("signal_scope_identity");
    this.identity = identity;
    this.#parent = parent;
    this.#onChange = onChange;
    for (const declaration of declarations) {
      if (this.#cells.has(declaration.name)) throw new Error("signal_declaration_duplicate");
      this.#cells.set(declaration.name, {
        initial: declaration.initial,
        value: declaration.initial,
      });
    }
    Object.seal(this);
  }

  get(name: string): SignalValue {
    this.#assertActive();
    const owner = this.owner(name);
    const cell = owner.#cells.get(name);
    if (cell === undefined) throw new Error("signal_missing");
    return cell.value;
  }

  owner(name: string): LocalSignalScope {
    this.#assertActive();
    if (this.#cells.has(name)) return this;
    let scope = this.#parent;
    let depth = 1;
    while (scope !== null && depth < 32) {
      if (scope.#cells.has(name)) return scope;
      scope = scope.#parent;
      depth += 1;
    }
    if (scope !== null) throw new Error("signal_scope_cycle");
    throw new Error("signal_missing");
  }

  set(name: string, value: SignalValue): void {
    this.#assertActive();
    const owner = this.owner(name);
    owner.#setOwned(name, value);
  }

  setDeclared(name: string, value: SignalValue): void {
    this.#assertActive();
    if (!this.#cells.has(name)) throw new Error("signal_missing");
    this.#setOwned(name, value);
  }

  toggle(name: string): void {
    const current = this.get(name);
    if (typeof current !== "boolean") throw new Error("signal_boolean_required");
    this.set(name, !current);
  }

  reset(name?: string): void {
    this.#assertActive();
    if (name !== undefined) {
      const owner = this.owner(name);
      const cell = owner.#cells.get(name);
      if (cell === undefined) throw new Error("signal_missing");
      owner.#setOwned(name, cell.initial);
      return;
    }
    this.batch(() => {
      for (const [cellName, cell] of this.#cells) this.#setOwned(cellName, cell.initial);
    });
  }

  batch(update: () => void): void {
    this.#assertActive();
    this.#batchDepth += 1;
    try {
      update();
    } finally {
      this.#batchDepth -= 1;
      if (this.#batchDepth === 0) this.#flush();
    }
  }

  values(): Readonly<Record<string, SignalValue>> {
    this.#assertActive();
    const values: Record<string, SignalValue> = Object.create(null) as Record<string, SignalValue>;
    for (const [name, cell] of this.#cells) values[name] = cell.value;
    return Object.freeze(values);
  }

  restore(values: Readonly<Record<string, SignalValue>>): boolean {
    this.#assertActive();
    const names = Object.keys(values);
    if (
      names.length !== this.#cells.size ||
      names.some((name) => {
        const cell = this.#cells.get(name);
        const value = values[name];
        return cell === undefined || value === undefined || !compatible(cell.initial, value);
      })
    ) {
      return false;
    }
    this.batch(() => {
      for (const name of names) {
        const value = values[name];
        if (value !== undefined) this.#setOwned(name, value);
      }
    });
    return true;
  }

  dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    this.#pending.clear();
    this.#cells.clear();
  }

  #setOwned(name: string, value: SignalValue): void {
    this.#assertActive();
    const cell = this.#cells.get(name);
    if (cell === undefined) throw new Error("signal_missing");
    if (!compatible(cell.initial, value)) throw new Error("signal_type_mismatch");
    if (Object.is(cell.value, value)) return;
    cell.value = value;
    this.#pending.add(name);
    if (this.#batchDepth === 0) this.#flush();
  }

  #flush(): void {
    if (this.#pending.size === 0) return;
    const names = Object.freeze([...this.#pending]);
    this.#pending.clear();
    this.#onChange(this, names);
  }

  #assertActive(): void {
    if (this.#disposed) throw new Error("signal_scope_disposed");
  }
}
