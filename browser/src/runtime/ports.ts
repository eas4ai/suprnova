export interface RuntimeClock {
  now(): number;
}

export interface RuntimeRandomness {
  randomBytes(length: number): Uint8Array;
}

export interface TransportPort {
  fetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response>;
}

export interface NavigationPort {
  assign(target: URL): void;
  replace(target: URL): void;
  reload(): void;
}

export interface RuntimeObserverFactory {
  mutation(callback: MutationCallback): MutationObserver;
  intersection(
    callback: IntersectionObserverCallback,
    options?: IntersectionObserverInit,
  ): IntersectionObserver | null;
}

export interface RuntimeScheduler {
  microtask(callback: VoidFunction): void;
  animationFrame(callback: FrameRequestCallback): number;
  cancelAnimationFrame(handle: number): void;
  timeout(callback: VoidFunction, milliseconds: number): number;
  clearTimeout(handle: number): void;
}

export interface RuntimeFeatures {
  prefersReducedMotion(): boolean;
  supportsViewTransitions(): boolean;
  supportsSpeculationRules(): boolean;
}

export interface RuntimePorts {
  readonly clock: RuntimeClock;
  readonly randomness: RuntimeRandomness;
  readonly transport: TransportPort;
  readonly navigation: NavigationPort;
  readonly observers: RuntimeObserverFactory;
  readonly scheduler: RuntimeScheduler;
  readonly features: RuntimeFeatures;
}

export interface RuntimePortOverrides {
  readonly clock?: RuntimeClock;
  readonly randomness?: RuntimeRandomness;
  readonly transport?: TransportPort;
  readonly navigation?: NavigationPort;
  readonly observers?: RuntimeObserverFactory;
  readonly scheduler?: RuntimeScheduler;
  readonly features?: RuntimeFeatures;
}

export function resolveRuntimePorts(
  defaults: RuntimePorts,
  overrides: RuntimePortOverrides,
): RuntimePorts {
  return Object.freeze({
    clock: overrides.clock ?? defaults.clock,
    randomness: overrides.randomness ?? defaults.randomness,
    transport: overrides.transport ?? defaults.transport,
    navigation: overrides.navigation ?? defaults.navigation,
    observers: overrides.observers ?? defaults.observers,
    scheduler: overrides.scheduler ?? defaults.scheduler,
    features: overrides.features ?? defaults.features,
  });
}

export function productionRuntimePorts(window: Window): RuntimePorts {
  const platform = window as Window & typeof globalThis;
  const document = window.document;
  return {
    clock: { now: () => Date.now() },
    randomness: {
      randomBytes(length) {
        if (!Number.isSafeInteger(length) || length < 1 || length > 4_096) {
          throw new RangeError("runtime_random_length");
        }
        return window.crypto.getRandomValues(new Uint8Array(length));
      },
    },
    transport: { fetch: window.fetch.bind(window) },
    navigation: {
      assign(target) {
        window.location.assign(target.href);
      },
      replace(target) {
        window.history.replaceState(null, "", target.href);
      },
      reload() {
        window.location.reload();
      },
    },
    observers: {
      mutation: (callback) => new platform.MutationObserver(callback),
      intersection: (callback, options) =>
        typeof platform.IntersectionObserver === "function"
          ? new platform.IntersectionObserver(callback, options)
          : null,
    },
    scheduler: {
      microtask(callback) {
        window.queueMicrotask(callback);
      },
      animationFrame: (callback) => window.requestAnimationFrame(callback),
      cancelAnimationFrame(handle) {
        window.cancelAnimationFrame(handle);
      },
      timeout: (callback, milliseconds) => window.setTimeout(callback, milliseconds),
      clearTimeout(handle) {
        window.clearTimeout(handle);
      },
    },
    features: {
      prefersReducedMotion: () => window.matchMedia("(prefers-reduced-motion: reduce)").matches,
      supportsViewTransitions: () => typeof document.startViewTransition === "function",
      supportsSpeculationRules: () =>
        typeof platform.HTMLScriptElement.supports === "function" &&
        platform.HTMLScriptElement.supports("speculationrules"),
    },
  };
}
