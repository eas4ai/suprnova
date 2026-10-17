import type {
  AsyncRuntimeIslandPort,
  FreshRenderCompletion,
  RegisteredBrowserEventCapability,
  PartiallyDispatchedBrowserEvent,
} from "../features/contract.js";
import type { AsyncPayload, ValidatedAsyncEnvelope } from "./types.js";

export type AsyncDispatchDisposition =
  | "queued"
  | "coalesced"
  | "dispatched"
  | "signal_updated"
  | "observed"
  | "closed:server_shutdown"
  | "closed:subscription_retired"
  | "closed:stream_completed"
  | "degraded:authorization_lost"
  | "degraded:replay_unavailable"
  | "degraded:backpressure"
  | "degraded:stream_unavailable"
  | "exhausted"
  | PartiallyDispatchedBrowserEvent
  | "rejected";

export interface AsyncEnvelopeDispatcher {
  dispatch(
    envelope: ValidatedAsyncEnvelope,
    completion?: (outcome: FreshRenderCompletion) => void,
  ): AsyncDispatchDisposition;
}

type EventPayload = Extract<AsyncPayload, { kind: "browser_event" }>;
type PresentationSignalPayload = Extract<AsyncPayload, { kind: "presentation_signal" }>;

function exactPayload(value: unknown, keys: readonly string[]): value is Record<string, unknown> {
  if (value === null || typeof value !== "object" || Array.isArray(value)) return false;
  const present = Reflect.ownKeys(value);
  return (
    present.length === keys.length &&
    keys.every((key) => Object.prototype.hasOwnProperty.call(value, key))
  );
}

function unsupported(): never {
  throw new Error("unsupported_async_payload");
}

/**
 * Routes a validated async envelope through the three core-owned presentation
 * capabilities. It intentionally exposes no action, effect, call, HTML,
 * snapshot, revision, or component-state seam.
 */
export class AsyncDispatcher implements AsyncEnvelopeDispatcher {
  readonly #capability: () => RegisteredBrowserEventCapability | null;
  readonly #island: AsyncRuntimeIslandPort;

  constructor(
    island: AsyncRuntimeIslandPort,
    capability: () => RegisteredBrowserEventCapability | null,
  ) {
    this.#island = island;
    this.#capability = capability;
  }

  dispatch(
    envelope: ValidatedAsyncEnvelope,
    completion?: (outcome: FreshRenderCompletion) => void,
  ): AsyncDispatchDisposition {
    let payload: unknown;
    let kind: unknown;
    try {
      payload = Reflect.get(envelope, "payload");
      kind = Reflect.get(payload as object, "kind");
    } catch {
      return unsupported();
    }
    switch (kind) {
      case "refresh":
        if (!exactPayload(payload, ["kind", "name"]) || payload["name"] !== "refresh") {
          return unsupported();
        }
        return this.#refresh(envelope.subscriptionId, completion);
      case "browser_event":
        if (
          !exactPayload(payload, ["event", "kind", "payload", "schema_version", "target"]) ||
          typeof payload["event"] !== "string" ||
          !Number.isSafeInteger(payload["schema_version"]) ||
          typeof payload["target"] !== "string"
        ) {
          return unsupported();
        }
        return this.#browserEvent(payload as unknown as EventPayload);
      case "presentation_signal":
        if (
          !exactPayload(payload, ["kind", "name", "scope", "value"]) ||
          typeof payload["name"] !== "string" ||
          typeof payload["scope"] !== "string"
        ) {
          return unsupported();
        }
        return this.#presentationSignal(payload as unknown as PresentationSignalPayload);
      case "heartbeat":
        if (!exactPayload(payload, ["kind"])) return unsupported();
        return "observed";
      case "complete":
        if (
          !exactPayload(payload, ["kind", "reason"]) ||
          (payload["reason"] !== "server_shutdown" &&
            payload["reason"] !== "subscription_retired" &&
            payload["reason"] !== "stream_completed")
        ) {
          return unsupported();
        }
        return `closed:${payload["reason"]}`;
      case "error":
        if (
          !exactPayload(payload, ["code", "kind"]) ||
          (payload["code"] !== "authorization_lost" &&
            payload["code"] !== "replay_unavailable" &&
            payload["code"] !== "backpressure" &&
            payload["code"] !== "stream_unavailable")
        ) {
          return unsupported();
        }
        return `degraded:${payload["code"]}`;
      default:
        return unsupported();
    }
  }

  #browserEvent(event: EventPayload): AsyncDispatchDisposition {
    let capability: RegisteredBrowserEventCapability | null;
    try {
      capability = this.#capability();
    } catch {
      return "rejected";
    }
    if (capability === null) return "rejected";
    try {
      const disposition = this.#island.dispatchRegisteredEvent(capability, {
        event: event.event,
        payload: event.payload,
        schemaVersion: event.schema_version,
        target: event.target,
      });
      return disposition === "dispatched"
        ? "dispatched"
        : typeof disposition === "object"
          ? disposition
          : "rejected";
    } catch {
      return "rejected";
    }
  }

  #presentationSignal(signal: PresentationSignalPayload): AsyncDispatchDisposition {
    try {
      this.#island.writePresentationSignal(signal.scope, signal.name, signal.value);
      return "signal_updated";
    } catch {
      return "rejected";
    }
  }

  #refresh(
    subscriptionId: string,
    completion?: (outcome: FreshRenderCompletion) => void,
  ): AsyncDispatchDisposition {
    let completed = false;
    const observe = (outcome: FreshRenderCompletion): void => {
      if (completed) return;
      completed = true;
      completion?.(outcome);
    };
    try {
      const disposition = this.#island.enqueueFreshRender("stream", observe, subscriptionId);
      return disposition === "queued" || disposition === "coalesced" || disposition === "exhausted"
        ? disposition
        : "rejected";
    } catch {
      return "rejected";
    }
  }
}
