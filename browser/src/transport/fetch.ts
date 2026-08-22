import type { RuntimeScheduler, TransportPort } from "../runtime/ports.js";
import type { IslandRecord } from "../islands/record.js";
import type { RuntimeDiagnosticInput, RuntimeDiagnostics } from "../runtime/diagnostics.js";
import type { RuntimePorts } from "../runtime/ports.js";
import type { RuntimeConfig } from "../runtime/types.js";
import type { SchedulerTicket } from "../scheduler/types.js";
import type { BuiltLiveRequest } from "./request.js";
import { LiveRequestBuilder } from "./request.js";
import { readLiveResponse } from "./response.js";
import { retryLiveRequest, type RetryPolicy } from "./retry.js";
import { LiveTransportError, TransportAttemptState, type LiveTransportResponse } from "./state.js";

export { LiveTransportError } from "./state.js";

export function liveMediaType(version: 1 | 2): string {
  return `application/vnd.suprnova.live+json; charset=utf-8; version=${String(version)}`;
}

export interface LiveFetchOptions {
  readonly endpoint: URL;
  readonly credentials: "same-origin" | "include";
  readonly requestTimeoutMs: number;
  readonly maxResponseBytes: number;
  readonly transport: TransportPort;
  readonly scheduler: RuntimeScheduler;
  readonly isOnline: () => boolean;
  readonly signal?: AbortSignal;
}

function endpoint(value: URL): URL {
  if (
    !(value instanceof URL) ||
    !["http:", "https:"].includes(value.protocol) ||
    value.username.length !== 0 ||
    value.password.length !== 0 ||
    value.hash.length !== 0
  ) {
    throw new LiveTransportError("unsafe_endpoint");
  }
  return new URL(value.href);
}

function safeClear(scheduler: RuntimeScheduler, handle: number | null): void {
  if (handle === null) return;
  try {
    scheduler.clearTimeout(handle);
  } catch {
    // Cleanup ports cannot rewrite a completed transport outcome.
  }
}

function aborted(signal: AbortSignal | undefined): boolean {
  return signal?.aborted === true;
}

export async function fetchLiveRequest(
  request: BuiltLiveRequest,
  options: LiveFetchOptions,
): Promise<LiveTransportResponse> {
  const state = new TransportAttemptState();
  const controller = new AbortController();
  const onAbort = (): void => {
    state.userAbort();
    controller.abort();
  };
  if (aborted(options.signal)) throw new LiveTransportError("aborted");
  options.signal?.addEventListener("abort", onAbort, { once: true });
  let timeoutHandle: number | null = null;
  try {
    timeoutHandle = options.scheduler.timeout(() => {
      state.timeout();
      controller.abort();
    }, options.requestTimeoutMs);
    state.beginFetch();
    const response = await options.transport.fetch(endpoint(options.endpoint), {
      body: request.text,
      cache: "no-store",
      credentials: options.credentials,
      headers: {
        Accept: request.mediaType,
        "Content-Type": request.mediaType,
      },
      method: "POST",
      redirect: "error",
      signal: controller.signal,
    });
    state.beginRead();
    const result = await readLiveResponse(request, response, options.maxResponseBytes);
    if (aborted(options.signal)) throw new LiveTransportError("aborted");
    if (controller.signal.aborted) throw state.interruption(options.isOnline);
    state.settle();
    return result;
  } catch (error: unknown) {
    state.settle();
    if (error instanceof LiveTransportError) throw error;
    throw state.interruption(options.isOnline);
  } finally {
    safeClear(options.scheduler, timeoutHandle);
    options.signal?.removeEventListener("abort", onAbort);
  }
}

const DEFAULT_RETRY_POLICY: RetryPolicy = Object.freeze({
  baseDelayMs: 100,
  jitterRatio: 0.2,
  maximumAttempts: 3,
  maximumDelayMs: 1_000,
  retryableStatuses: Object.freeze([502, 503, 504]),
});

export function transportFailureDiagnostic(
  failure: LiveTransportError,
): RuntimeDiagnosticInput | null {
  if (failure.kind === "aborted") return null;
  return {
    code: "transport_failed",
    detailCode:
      failure.kind === "protocol" || failure.kind === "correlation"
        ? "invalid_response"
        : failure.kind === "unsafe_endpoint"
          ? "unsafe_endpoint"
          : "network_failure",
    phase: "transport",
    severity: "error",
  };
}

interface IslandTransportWork {
  readonly controllers: Map<SchedulerTicket, AbortController>;
  readonly responses: Map<SchedulerTicket, CompletedLiveTransport>;
}

export interface CompletedLiveTransport {
  readonly request: BuiltLiveRequest;
  readonly response: LiveTransportResponse;
}

export type LiveResponseObserver = (record: IslandRecord, ticket: SchedulerTicket) => void;

export class LiveTransportCoordinator {
  readonly #config: RuntimeConfig;
  readonly #ports: RuntimePorts;
  readonly #diagnostics: RuntimeDiagnostics;
  readonly #builder = new LiveRequestBuilder();
  readonly #work = new Map<IslandRecord, IslandTransportWork>();
  readonly #responseObserver: LiveResponseObserver;
  #disposed = false;

  constructor(
    config: RuntimeConfig,
    ports: RuntimePorts,
    diagnostics: RuntimeDiagnostics,
    responseObserver: LiveResponseObserver = () => undefined,
  ) {
    this.#config = config;
    this.#ports = ports;
    this.#diagnostics = diagnostics;
    this.#responseObserver = responseObserver;
  }

  connect(record: IslandRecord): void {
    if (this.#disposed || this.#work.has(record)) {
      throw new Error("transport_coordinator_connect_rejected");
    }
    const work: IslandTransportWork = {
      controllers: new Map(),
      responses: new Map(),
    };
    this.#work.set(record, work);
    record.attachScheduleObserver(() => {
      this.#pump(record);
    });
    record.onDispose(() => {
      this.#retire(record);
    });
    this.#pump(record);
  }

  takeResponse(record: IslandRecord, ticket: SchedulerTicket): CompletedLiveTransport | null {
    const work = this.#work.get(record);
    const response = work?.responses.get(ticket) ?? null;
    work?.responses.delete(ticket);
    return response;
  }

  resume(record: IslandRecord): void {
    this.#pump(record);
  }

  dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    for (const record of [...this.#work.keys()]) this.#retire(record);
  }

  #pump(record: IslandRecord): void {
    if (this.#disposed || !record.active()) return;
    const work = this.#work.get(record);
    if (work === undefined) return;
    for (const ticket of record.scheduler.ready()) {
      const controller = new AbortController();
      if (
        record.scheduler.start(ticket, () => {
          controller.abort();
        }) !== "accepted"
      ) {
        continue;
      }
      work.controllers.set(ticket, controller);
      void this.#run(record, ticket, controller);
    }
  }

  async #run(
    record: IslandRecord,
    ticket: SchedulerTicket,
    controller: AbortController,
  ): Promise<void> {
    const work = this.#work.get(record);
    if (work === undefined) return;
    try {
      const protocolVersion = Math.max(
        this.#config.protocol.minimum,
        record.metadata.protocolMinimum,
      ) as 1 | 2;
      const request = await this.#builder.build({
        intent: ticket.intent,
        protocolVersion,
        randomness: this.#ports.randomness,
      });
      const result = await retryLiveRequest(request, {
        attempt: (candidate, signal) =>
          fetchLiveRequest(candidate, {
            credentials: this.#config.credentials,
            endpoint: this.#config.endpoint,
            isOnline: () => this.#ports.connectivity.isOnline(),
            maxResponseBytes: this.#config.maxResponseBytes,
            requestTimeoutMs: this.#config.requestTimeoutMs,
            scheduler: this.#ports.scheduler,
            ...(signal === undefined ? {} : { signal }),
            transport: this.#ports.transport,
          }),
        clock: this.#ports.clock,
        isOnline: () => this.#ports.connectivity.isOnline(),
        jitter: () => this.#jitter(),
        onAttempt: () => {
          record.scheduler.setTransportFeedback(ticket, { offline: false, retrying: false });
        },
        onRetry: (failure) => {
          record.scheduler.setTransportFeedback(ticket, {
            offline: failure.kind === "offline",
            retrying: true,
          });
        },
        policy: DEFAULT_RETRY_POLICY,
        scheduler: this.#ports.scheduler,
        signal: controller.signal,
      });
      work.controllers.delete(ticket);
      if (record.scheduler.settleTransport(ticket) === "accepted") {
        work.responses.set(ticket, Object.freeze({ request, response: result }));
        try {
          this.#responseObserver(record, ticket);
        } catch {
          record.scheduler.finish(ticket, "rejected");
          this.#diagnostics.record({
            code: "transport_failed",
            detailCode: "invalid_response",
            phase: "transport",
            severity: "error",
          });
          this.#pump(record);
        }
      }
    } catch (error: unknown) {
      work.controllers.delete(ticket);
      const failure =
        error instanceof LiveTransportError ? error : new LiveTransportError("network");
      record.scheduler.setTransportFeedback(ticket, {
        offline: failure.kind === "offline",
        retrying: false,
      });
      record.scheduler.finish(ticket, failure.kind === "aborted" ? "canceled" : "rejected");
      const diagnostic = transportFailureDiagnostic(failure);
      if (diagnostic !== null) this.#diagnostics.record(diagnostic);
      this.#pump(record);
    }
  }

  #jitter(): number {
    const bytes = this.#ports.randomness.randomBytes(2);
    if (!(bytes instanceof Uint8Array) || bytes.byteLength !== 2) {
      throw new LiveTransportError("network");
    }
    return (((bytes[0] ?? 0) * 256 + (bytes[1] ?? 0)) / 65_535) * 2 - 1;
  }

  #retire(record: IslandRecord): void {
    const work = this.#work.get(record);
    if (work === undefined) return;
    this.#work.delete(record);
    for (const controller of work.controllers.values()) controller.abort();
    work.controllers.clear();
    work.responses.clear();
  }
}
