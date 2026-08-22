import { boundResourceLedger, type ResourceKind } from "./resources.js";

export interface LifecycleTestProbe {
  readonly counts: Readonly<Record<ResourceKind, number>>;
  readonly weak: Readonly<{ deref(): object | undefined }> | null;
}

type WeakReferenceConstructor = new (value: object) => Readonly<{ deref(): object | undefined }>;

export function lifecycleTestProbe(owner: object): LifecycleTestProbe {
  const ledger = boundResourceLedger(owner);
  if (ledger === null) throw new Error("lifecycle_test_probe_missing");
  const candidate: unknown = Reflect.get(globalThis, "WeakRef");
  const WeakReference =
    typeof candidate === "function" ? (candidate as WeakReferenceConstructor) : null;
  return Object.freeze({
    counts: ledger.counts(),
    weak: WeakReference === null ? null : new WeakReference(owner),
  });
}
