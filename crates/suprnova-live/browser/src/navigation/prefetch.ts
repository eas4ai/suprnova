import type { NativeNavigationIntent } from "./eligibility.js";

export type PrefetchVariance = "credentials" | "tenant" | "principal" | "locale";
export type PrefetchCachePolicy = "public" | "private" | "no-store";
export type PrefetchIneligibleReason =
  | "method"
  | "cross_origin"
  | "credentials_variance"
  | "tenant_variance"
  | "principal_variance"
  | "locale_variance"
  | "flash"
  | "no_store"
  | "private"
  | "data_saver"
  | "redirect"
  | "hidden"
  | "not_explicit";

export interface PrefetchContext {
  readonly current: URL;
  readonly explicit: boolean;
  readonly cachePolicy: PrefetchCachePolicy;
  readonly variesBy: readonly PrefetchVariance[];
  readonly consumesFlash: boolean;
  readonly saveData: boolean;
  readonly redirectProne: boolean;
  readonly hidden: boolean;
}

export type PrefetchEligibility =
  Readonly<{ eligible: true }> | Readonly<{ eligible: false; reason: PrefetchIneligibleReason }>;

export interface PrefetchEmission {
  cancel(): void;
}

export interface PrefetchHost {
  emit(kind: "link" | "speculation", target: URL): PrefetchEmission;
}

export class BrowserPrefetchHost implements PrefetchHost {
  readonly #document: Document;

  constructor(document: Document) {
    this.#document = document;
  }

  emit(kind: "link" | "speculation", target: URL): PrefetchEmission {
    if (kind === "speculation") return this.#speculation(target);
    const link = this.#document.createElement("link");
    link.rel = "prefetch";
    link.href = target.href;
    link.setAttribute("data-suprnova-live-prefetch-resource", "link");
    this.#document.head.append(link);
    return Object.freeze({
      cancel: () => {
        link.remove();
      },
    });
  }

  #speculation(target: URL): PrefetchEmission {
    const script = this.#document.createElement("script");
    script.type = "speculationrules";
    script.textContent = JSON.stringify({
      prefetch: [{ source: "list", urls: [target.href] }],
    });
    script.setAttribute("data-suprnova-live-prefetch-resource", "speculation");
    this.#document.head.append(script);
    return Object.freeze({
      cancel: () => {
        script.remove();
      },
    });
  }
}

export type PrefetchRequestDisposition = "emitted" | "duplicate" | "capacity" | "ineligible";

function ineligible(reason: PrefetchIneligibleReason): PrefetchEligibility {
  return Object.freeze({ eligible: false, reason });
}

export function prefetchEligibility(
  intent: NativeNavigationIntent,
  context: PrefetchContext,
): PrefetchEligibility {
  if (intent.method !== "GET" && intent.method !== "HEAD") return ineligible("method");
  if (intent.prefetch === "none") return ineligible("not_explicit");
  if (intent.target.origin !== context.current.origin) return ineligible("cross_origin");
  if (!context.explicit) return ineligible("not_explicit");
  for (const variance of context.variesBy) return ineligible(`${variance}_variance`);
  if (context.consumesFlash) return ineligible("flash");
  if (context.cachePolicy === "no-store") return ineligible("no_store");
  if (context.cachePolicy === "private") return ineligible("private");
  if (context.saveData) return ineligible("data_saver");
  if (context.redirectProne) return ineligible("redirect");
  if (context.hidden) return ineligible("hidden");
  return Object.freeze({ eligible: true });
}

export interface PrefetchCoordinatorOptions {
  readonly host: PrefetchHost;
  readonly maxConcurrent: number;
}

export class PrefetchCoordinator {
  readonly #host: PrefetchHost;
  readonly #maxConcurrent: number;
  readonly #active = new Map<string, Readonly<{ emission: PrefetchEmission; target: string }>>();
  readonly #targets = new Map<string, string>();
  #disposed = false;

  constructor(options: PrefetchCoordinatorOptions) {
    if (
      !Number.isSafeInteger(options.maxConcurrent) ||
      options.maxConcurrent < 1 ||
      options.maxConcurrent > 8
    ) {
      throw new RangeError("prefetch_concurrency_limit");
    }
    this.#host = options.host;
    this.#maxConcurrent = options.maxConcurrent;
  }

  request(
    key: string,
    intent: NativeNavigationIntent,
    context: PrefetchContext,
  ): PrefetchRequestDisposition {
    if (this.#disposed || !prefetchEligibility(intent, context).eligible) {
      return "ineligible";
    }
    if (this.#active.has(key) || this.#targets.has(intent.target.href)) return "duplicate";
    if (this.#active.size >= this.#maxConcurrent) return "capacity";
    const kind = intent.prefetch === "speculation" ? "speculation" : "link";
    const emission = this.#host.emit(kind, intent.target);
    this.#active.set(key, Object.freeze({ emission, target: intent.target.href }));
    this.#targets.set(intent.target.href, key);
    return "emitted";
  }

  cancel(key: string): boolean {
    const active = this.#active.get(key);
    if (active === undefined) return false;
    this.#active.delete(key);
    if (this.#targets.get(active.target) === key) this.#targets.delete(active.target);
    try {
      active.emission.cancel();
    } catch {
      // Native resource cancellation is best effort and never alters navigation.
    }
    return true;
  }

  removed(key: string): void {
    this.cancel(key);
  }

  cancelAll(): void {
    for (const key of [...this.#active.keys()]) this.cancel(key);
  }

  dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    this.cancelAll();
  }
}
