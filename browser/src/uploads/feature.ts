import {
  defineUploadsFeature,
  type FeatureIslandController,
  type RuntimeFeature,
  type RuntimeFeatureDocumentContext,
  type UploadsRuntimeFeatureDefinition,
  type UploadsRuntimeIslandPort,
} from "../features/contract.js";
import { parseFeatureDirective } from "../features/directive-parser.js";
import { UploadManager } from "./manager.js";
import { captureUploadMorph, reconcileUploadMorph, type UploadMorphContinuity } from "./morph.js";
import { validateUploadTransportRequest } from "./protocol.js";
import {
  createUploadProgressView,
  UploadProgressPresenter,
  type UploadProgressView,
} from "./progress.js";
import {
  DEFAULT_UPLOAD_CHUNK_BYTES,
  MAX_UPLOAD_FILES_PER_DOCUMENT,
  type UploadConnectivity,
  type UploadApplicationPort,
  type UploadManagerOptions,
  type UploadRandomness,
  type UploadTransport,
  type UploadTransportRequest,
  type UploadTransportResponse,
  type UploadHandle,
} from "./types.js";

const DEFAULT_UPLOAD_ENDPOINT = "/__live/upload";
const DEFAULT_ACTIVE_UPLOADS = 4;
const DEFAULT_MANAGER_BYTES = 256 * 1024;
const MAX_UPLOAD_RESPONSE_BYTES = 16 * 1024;

export interface UploadFeatureOptions {
  readonly application?: UploadApplicationPort;
  readonly chunkBytes?: number;
  readonly connectivity?: UploadConnectivity;
  readonly maxActive?: number;
  readonly maxItems?: number;
  readonly maxQueueBytes?: number;
  readonly randomness?: UploadRandomness;
  readonly transport?: UploadTransport;
}

export interface UploadResumeRequest {
  readonly field: string;
  readonly file: File;
  readonly handle: UploadHandle;
  readonly input: HTMLInputElement;
  readonly island: Element;
}

interface UploadFeatureOwner {
  manager: UploadManager | null;
  ports: WeakMap<Element, UploadsRuntimeIslandPort>;
}

let defaultConfiguration: UploadFeatureOptions = Object.freeze({});
let defaultConfigurationLocked = false;
const defaultOwner: UploadFeatureOwner = {
  manager: null,
  ports: new WeakMap<Element, UploadsRuntimeIslandPort>(),
};

class UploadHttpError extends Error {
  constructor(readonly code: "upload_expired" | "upload_transport_failed") {
    super(code);
    this.name = "UploadHttpError";
  }
}

function authorization(grant: string): string {
  return `SuprnovaUpload ${grant}`;
}

function controlBody(request: UploadTransportRequest): Readonly<Record<string, unknown>> {
  switch (request.operation) {
    case "create":
      return Object.freeze({
        field: request.field,
        file: request.file,
        idempotency_key: request.idempotencyKey,
        island: request.island,
        operation: request.operation,
        protocol_version: 1,
      });
    case "complete":
      return Object.freeze({
        expected_revision: request.expectedRevision,
        handle: request.handle,
        idempotency_key: request.idempotencyKey,
        operation: request.operation,
        protocol_version: 1,
        whole_checksum: request.wholeChecksum,
      });
    case "cancel":
      return Object.freeze({
        expected_revision: request.expectedRevision,
        handle: request.handle,
        idempotency_key: request.idempotencyKey,
        operation: request.operation,
        protocol_version: 1,
      });
    case "status":
      return Object.freeze({
        handle: request.handle,
        operation: request.operation,
        protocol_version: 1,
      });
    case "put_chunk":
      return Object.freeze({});
  }
}

async function boundedResponse(response: Response): Promise<UploadTransportResponse> {
  const declaredLength = response.headers.get("Content-Length");
  if (
    declaredLength !== null &&
    (!/^(?:0|[1-9][0-9]*)$/u.test(declaredLength) ||
      Number(declaredLength) > MAX_UPLOAD_RESPONSE_BYTES)
  ) {
    throw new UploadHttpError("upload_transport_failed");
  }
  const reader = response.body?.getReader();
  if (reader === undefined) throw new UploadHttpError("upload_transport_failed");
  const bytes = new Uint8Array(MAX_UPLOAD_RESPONSE_BYTES);
  let length = 0;
  for (;;) {
    const item = await reader.read();
    if (item.done) break;
    if (item.value.byteLength > MAX_UPLOAD_RESPONSE_BYTES - length) {
      await reader.cancel();
      throw new UploadHttpError("upload_transport_failed");
    }
    bytes.set(item.value, length);
    length += item.value.byteLength;
  }
  let value: unknown;
  try {
    value = JSON.parse(
      new TextDecoder("utf-8", { fatal: true }).decode(bytes.subarray(0, length)),
    ) as unknown;
  } catch {
    throw new UploadHttpError("upload_transport_failed");
  }
  if ((typeof value !== "object" && typeof value !== "function") || value === null) {
    throw new UploadHttpError("upload_transport_failed");
  }
  return value as UploadTransportResponse;
}

export class FetchUploadTransport implements UploadTransport {
  readonly #fetch: typeof globalThis.fetch;

  constructor(fetchPort: typeof globalThis.fetch) {
    if (typeof fetchPort !== "function") throw new Error("upload_transport_configuration_invalid");
    this.#fetch = fetchPort;
  }

  async send(request: UploadTransportRequest): Promise<UploadTransportResponse> {
    validateUploadTransportRequest(request);
    const headers = new Headers({ Accept: "application/json", "X-Suprnova-Live": "upload-v1" });
    let body: BodyInit;
    if (request.operation === "put_chunk") {
      headers.set("Authorization", authorization(request.grant));
      headers.set("Content-Type", "application/octet-stream");
      headers.set("X-Suprnova-Upload-Checksum", request.checksum);
      headers.set("X-Suprnova-Upload-Chunk", String(request.chunkIndex));
      headers.set("X-Suprnova-Upload-Handle", request.handle);
      headers.set("X-Suprnova-Upload-Idempotency", request.idempotencyKey);
      headers.set("X-Suprnova-Upload-Operation", request.operation);
      headers.set("X-Suprnova-Upload-Revision", request.expectedRevision);
      body = request.bytes;
    } else {
      headers.set("Content-Type", "application/json");
      if (request.operation !== "create") {
        headers.set("Authorization", authorization(request.grant));
      }
      body = JSON.stringify(controlBody(request));
    }
    const response = await this.#fetch(DEFAULT_UPLOAD_ENDPOINT, {
      body,
      cache: "no-store",
      credentials: "same-origin",
      headers,
      method: "POST",
      redirect: "error",
      referrerPolicy: "same-origin",
      signal: request.signal,
    });
    if (!response.ok) {
      throw new UploadHttpError(
        response.status === 404 || response.status === 410
          ? "upload_expired"
          : "upload_transport_failed",
      );
    }
    return boundedResponse(response);
  }
}

class BrowserConnectivity implements UploadConnectivity {
  online(): boolean {
    return typeof navigator === "undefined" || navigator.onLine;
  }
}

class BrowserRandomness implements UploadRandomness {
  idempotencyKey(): string {
    const bytes = globalThis.crypto.getRandomValues(new Uint8Array(18));
    let binary = "";
    for (const byte of bytes) binary += String.fromCodePoint(byte);
    return `upload-${btoa(binary).replace(/\+/gu, "-").replace(/\//gu, "_").replace(/=+$/u, "")}`;
  }
}

function snapshotOptions(options: UploadFeatureOptions): UploadFeatureOptions {
  return Object.freeze({
    ...(options.application === undefined ? {} : { application: options.application }),
    ...(options.chunkBytes === undefined ? {} : { chunkBytes: options.chunkBytes }),
    ...(options.connectivity === undefined ? {} : { connectivity: options.connectivity }),
    ...(options.maxActive === undefined ? {} : { maxActive: options.maxActive }),
    ...(options.maxItems === undefined ? {} : { maxItems: options.maxItems }),
    ...(options.maxQueueBytes === undefined ? {} : { maxQueueBytes: options.maxQueueBytes }),
    ...(options.randomness === undefined ? {} : { randomness: options.randomness }),
    ...(options.transport === undefined ? {} : { transport: options.transport }),
  });
}

function resolveOptions(options: UploadFeatureOptions): UploadManagerOptions {
  const fetchPort = globalThis.fetch;
  return Object.freeze({
    ...(options.application === undefined ? {} : { application: options.application }),
    chunkBytes: options.chunkBytes ?? DEFAULT_UPLOAD_CHUNK_BYTES,
    connectivity: options.connectivity ?? new BrowserConnectivity(),
    maxActive: options.maxActive ?? DEFAULT_ACTIVE_UPLOADS,
    maxItems: options.maxItems ?? MAX_UPLOAD_FILES_PER_DOCUMENT,
    maxQueueBytes: options.maxQueueBytes ?? DEFAULT_MANAGER_BYTES,
    randomness: options.randomness ?? new BrowserRandomness(),
    transport: options.transport ?? new FetchUploadTransport(fetchPort.bind(globalThis)),
  });
}

function report(context: RuntimeFeatureDocumentContext): void {
  try {
    context.diagnose("operation_rejected");
  } catch {
    // Optional diagnostics remain fixed and best-effort.
  }
}

function accessibleName(element: Element): boolean {
  try {
    return (
      (element.getAttribute("aria-label")?.trim().length ?? 0) > 0 ||
      (element.getAttribute("aria-labelledby")?.trim().length ?? 0) > 0 ||
      element.textContent.trim().length > 0
    );
  } catch {
    return false;
  }
}

function canceledProgress(): UploadProgressView {
  return Object.freeze({
    loadedBytes: 0,
    percent: null,
    state: "canceled",
    totalBytes: 0,
  });
}

export function connectUploadIsland(
  manager: UploadManager,
  context: RuntimeFeatureDocumentContext,
  port: UploadsRuntimeIslandPort,
): FeatureIslandController {
  const disposers: VoidFunction[] = [];
  const presenter = new UploadProgressPresenter();
  let ownerships = port.queryDirectiveOwnership(parseFeatureDirective);
  let continuity: UploadMorphContinuity | null = null;
  let disposed = false;

  const clearListeners = (): void => {
    for (let index = disposers.length - 1; index >= 0; index -= 1) disposers[index]?.();
    disposers.length = 0;
  };
  const progressRoots = (field: string): readonly Element[] =>
    ownerships.flatMap(({ directive, element }) =>
      directive.name === "progress" && directive.role === null && directive.value === field
        ? [element]
        : [],
    );
  const projectControls = (field: string, view: UploadProgressView | null): void => {
    for (const { directive, element } of ownerships) {
      if (directive.name !== "upload" || directive.value !== field || directive.role === null) {
        continue;
      }
      if (element.tagName.toUpperCase() !== "BUTTON") continue;
      const button = element as HTMLButtonElement;
      const state = view?.state ?? null;
      const disabled =
        directive.role === "retry"
          ? state !== "interrupted" && state !== "failed"
          : directive.role === "remove"
            ? state === null
            : state === null ||
              state === "finalized" ||
              state === "canceled" ||
              state === "expired";
      button.disabled = disabled;
      button.setAttribute("aria-disabled", disabled ? "true" : "false");
    }
  };
  const project = (): void => {
    const fields = new Set(
      ownerships.flatMap(({ directive }) =>
        directive.name === "progress" || directive.name === "upload" ? [directive.value] : [],
      ),
    );
    for (const field of fields) {
      const view = createUploadProgressView(manager.islandSnapshot(port, field).uploads);
      if (view !== null) {
        for (const root of progressRoots(field)) presenter.render(root, view);
      }
      projectControls(field, view);
    }
  };
  const installListeners = (): void => {
    clearListeners();
    for (const ownership of ownerships) {
      const { directive, element } = ownership;
      if (directive.name !== "upload") continue;
      let listener: EventListener;
      let eventType: "change" | "click";
      if (directive.role === null) {
        if (element.tagName.toUpperCase() !== "INPUT") {
          report(context);
          continue;
        }
        const input = element as HTMLInputElement;
        if (input.type.toLowerCase() !== "file") {
          report(context);
          continue;
        }
        eventType = "change";
        listener = (event) => {
          if (event.target !== input) return;
          const files = input.files === null ? [] : [...input.files];
          void manager.select({ field: directive.value, input, island: port }, files).catch(() => {
            report(context);
          });
        };
      } else {
        if (element.tagName.toUpperCase() !== "BUTTON" || !accessibleName(element)) {
          report(context);
          continue;
        }
        eventType = "click";
        listener = (event) => {
          if (!event.isTrusted) return;
          const operation =
            directive.role === "cancel"
              ? manager.cancel(port, directive.value)
              : directive.role === "retry"
                ? manager.retry(port, directive.value)
                : manager.remove(port, directive.value);
          void operation.catch(() => {
            report(context);
          });
        };
      }
      element.addEventListener(eventType, listener);
      disposers.push(() => {
        element.removeEventListener(eventType, listener);
      });
    }
  };
  const reconcileMorph = (): void => {
    if (continuity === null || disposed) return;
    ownerships = port.queryDirectiveOwnership(parseFeatureDirective);
    const compatible = reconcileUploadMorph(continuity, ownerships);
    continuity = null;
    const retired = manager.retireIncompatible(port, compatible);
    installListeners();
    project();
    for (const field of retired) {
      const view = canceledProgress();
      for (const root of progressRoots(field)) presenter.render(root, view);
      projectControls(field, view);
    }
  };

  installListeners();
  const stopObserving = manager.observeIsland(port, project);
  return Object.freeze({
    abortMorph() {
      reconcileMorph();
    },
    afterMorph() {
      reconcileMorph();
    },
    beforeMorph() {
      if (disposed) return;
      ownerships = port.queryDirectiveOwnership(parseFeatureDirective);
      continuity = captureUploadMorph(ownerships, manager.activeFields(port));
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      continuity = null;
      stopObserving();
      clearListeners();
      manager.retireIsland(port);
    },
  });
}

function defineConfiguredFeature(
  configuration: () => UploadFeatureOptions,
  owner?: UploadFeatureOwner,
): RuntimeFeature {
  const definition: UploadsRuntimeFeatureDefinition = Object.freeze({
    connectDocument(context: RuntimeFeatureDocumentContext) {
      const manager = new UploadManager(resolveOptions(configuration()));
      if (owner !== undefined) {
        defaultConfigurationLocked = true;
        owner.manager = manager;
        owner.ports = new WeakMap<Element, UploadsRuntimeIslandPort>();
      }
      let disposed = false;
      return Object.freeze({
        connectIsland(port: UploadsRuntimeIslandPort) {
          const controller = connectUploadIsland(manager, context, port);
          if (owner === undefined) return controller;
          owner.ports.set(port.element, port);
          return Object.freeze({
            abortMorph() {
              controller.abortMorph?.();
            },
            afterMorph() {
              controller.afterMorph?.();
            },
            beforeMorph() {
              controller.beforeMorph?.();
            },
            dispose() {
              if (owner.ports.get(port.element) === port) owner.ports.delete(port.element);
              controller.dispose();
            },
          });
        },
        dispose() {
          if (disposed) return;
          disposed = true;
          manager.dispose();
          if (owner?.manager === manager) {
            owner.manager = null;
            owner.ports = new WeakMap<Element, UploadsRuntimeIslandPort>();
          }
        },
        resume() {
          manager.resume();
        },
        suspend() {
          manager.suspend();
        },
      });
    },
  });
  return defineUploadsFeature(definition);
}

export function createUploadsFeature(options: UploadFeatureOptions = {}): RuntimeFeature {
  const configured = snapshotOptions(options);
  return defineConfiguredFeature(() => configured);
}

export function configureUploads(options: UploadFeatureOptions): void {
  if (defaultConfigurationLocked) throw new Error("upload_configuration_locked");
  defaultConfiguration = snapshotOptions(options);
  defaultConfigurationLocked = true;
}

export async function resumeUpload(request: UploadResumeRequest): Promise<void> {
  const manager = defaultOwner.manager;
  const island = defaultOwner.ports.get(request.island);
  if (manager === null || island === undefined) throw new Error("upload_resume_unavailable");
  await manager.reacquire(
    { field: request.field, input: request.input, island },
    request.file,
    request.handle,
  );
}

export const uploadsFeature: RuntimeFeature = defineConfiguredFeature(
  () => defaultConfiguration,
  defaultOwner,
);
