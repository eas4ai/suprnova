import { canonicalize, type JsonObject } from "../canonical.js";
import type { ServerIntent } from "../scheduler/intent.js";
import type { ValidatedChildDelivery } from "./types.js";

export class ChildParameterDeliveryState {
  #currentHash: string | null = null;
  readonly #pendingAuthorities = new Set<string>();

  track(parameterHash: string, intent: ServerIntent): boolean {
    const authority = canonicalize(intent.childParameters ?? null);
    if (this.#currentHash === parameterHash || this.#pendingAuthorities.has(authority))
      return false;
    this.#pendingAuthorities.add(authority);
    intent.onFinish((reason) => {
      if (!this.#pendingAuthorities.delete(authority)) return;
      if (reason === "accepted") this.#currentHash = parameterHash;
    });
    return true;
  }
}

export interface ChildDeliveryTarget {
  readonly instanceId: string;
  active(): boolean;
  queueParamsChanged(
    envelope: JsonObject,
    parentSnapshot: JsonObject,
    parameterHash: string,
  ): boolean;
}

export interface ChildDeliveryDirectory {
  find(instanceId: string): ChildDeliveryTarget | null;
}

export type ChildDeliveryDisposition = "queued" | "missing" | "retired" | "rejected";

export interface ChildDeliveryResult {
  readonly childInstance: string;
  readonly disposition: ChildDeliveryDisposition;
}

export function queueChildDeliveries(
  deliveries: readonly ValidatedChildDelivery[],
  parentSnapshot: JsonObject,
  directory: ChildDeliveryDirectory,
): readonly ChildDeliveryResult[] {
  return Object.freeze(
    deliveries.map((delivery) => {
      const target = directory.find(delivery.childInstance);
      let disposition: ChildDeliveryDisposition;
      if (target?.instanceId !== delivery.childInstance) disposition = "missing";
      else if (!target.active()) disposition = "retired";
      else {
        try {
          disposition = target.queueParamsChanged(
            delivery.envelope,
            parentSnapshot,
            delivery.parameterHash,
          )
            ? "queued"
            : "rejected";
        } catch {
          disposition = "rejected";
        }
      }
      return Object.freeze({ childInstance: delivery.childInstance, disposition });
    }),
  );
}
