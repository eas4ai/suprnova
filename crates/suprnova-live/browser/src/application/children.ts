import type { JsonObject } from "../canonical.js";
import type { ValidatedChildDelivery } from "./types.js";

export interface ChildDeliveryTarget {
  readonly instanceId: string;
  active(): boolean;
  queueParamsChanged(envelope: JsonObject, parameterHash: string): boolean;
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
  directory: ChildDeliveryDirectory,
): readonly ChildDeliveryResult[] {
  return Object.freeze(
    deliveries.map((delivery) => {
      const target = directory.find(delivery.childInstance);
      let disposition: ChildDeliveryDisposition;
      if (target?.instanceId !== delivery.childInstance) disposition = "missing";
      else if (!target.active()) disposition = "retired";
      else {
        disposition = target.queueParamsChanged(delivery.envelope, delivery.parameterHash)
          ? "queued"
          : "rejected";
      }
      return Object.freeze({ childInstance: delivery.childInstance, disposition });
    }),
  );
}
