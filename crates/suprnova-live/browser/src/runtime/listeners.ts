const MAX_DELEGATED_LISTENERS = 32;
const SAFE_EVENT_TYPE = /^[a-z][a-z0-9:-]{0,63}$/u;

interface ListenerDefinition {
  readonly type: string;
  readonly listener: EventListener;
  readonly options: Readonly<{ capture: boolean; passive: boolean }>;
}

/** Bounded document-level listener owner used by all delegated Live behavior. */
export class DelegatedListenerRegistry {
  readonly #target: EventTarget;
  readonly #definitions = new Map<string, ListenerDefinition>();
  #active = false;
  #disposed = false;

  constructor(target: EventTarget) {
    this.#target = target;
  }

  add(
    type: string,
    listener: EventListener,
    options: Readonly<{ capture?: boolean; passive?: boolean }> = {},
  ): VoidFunction {
    if (this.#disposed) throw new Error("listener_registry_disposed");
    const capture = options.capture ?? false;
    const definition = Object.freeze({
      type,
      listener,
      options: Object.freeze({ capture, passive: options.passive ?? false }),
    });
    const key = `${type}:${capture ? "capture" : "bubble"}`;
    if (
      !SAFE_EVENT_TYPE.test(type) ||
      this.#definitions.has(key) ||
      this.#definitions.size >= MAX_DELEGATED_LISTENERS
    ) {
      throw new Error("listener_registry_rejected");
    }
    this.#definitions.set(key, definition);
    if (this.#active) this.#attach(definition);
    let removed = false;
    return () => {
      if (removed) return;
      removed = true;
      const current = this.#definitions.get(key);
      if (current === undefined) return;
      if (this.#active) this.#detach(current);
      this.#definitions.delete(key);
    };
  }

  resume(): void {
    if (this.#disposed) throw new Error("listener_registry_disposed");
    if (this.#active) return;
    this.#active = true;
    for (const definition of this.#definitions.values()) this.#attach(definition);
  }

  suspend(): void {
    if (!this.#active) return;
    for (const definition of this.#definitions.values()) this.#detach(definition);
    this.#active = false;
  }

  dispose(): void {
    if (this.#disposed) return;
    this.suspend();
    this.#definitions.clear();
    this.#disposed = true;
  }

  #attach(definition: ListenerDefinition): void {
    this.#target.addEventListener(definition.type, definition.listener, definition.options);
  }

  #detach(definition: ListenerDefinition): void {
    this.#target.removeEventListener(definition.type, definition.listener, definition.options);
  }
}
