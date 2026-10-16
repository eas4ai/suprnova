import { describe, expect, it, vi } from "vitest";

import { BrowserAsyncTransportPorts } from "../src/async-updates/connections.js";
import type {
  BrowserAsyncTransportOptions,
  DocumentTransportConnectRequest,
  DocumentTransportFailure,
} from "../src/async-updates/connections.js";

interface FakeSocket {
  readonly close: (code?: number, reason?: string) => void;
  readonly send: (data: string) => void;
  onclose?: (event: unknown) => void;
  onerror?: VoidFunction;
  onopen?: VoidFunction;
}

function connectRequest(
  failed: (reason: DocumentTransportFailure) => void,
): DocumentTransportConnectRequest {
  return Object.freeze({
    authorization: Object.freeze({ kind: "session_cookie" as const }),
    failed,
    key: Object.freeze({
      authorizationScope: "document-scope",
      origin: "https://app.example.test",
      transport: "websocket" as const,
    }),
    message: vi.fn(),
    opened: vi.fn(),
    transportGeneration: 1,
  });
}

function openSocket(failed: (reason: DocumentTransportFailure) => void): FakeSocket {
  const sockets: FakeSocket[] = [];
  const options: BrowserAsyncTransportOptions = {
    eventSource: vi.fn<BrowserAsyncTransportOptions["eventSource"]>(),
    fetch: vi.fn<typeof globalThis.fetch>(),
    membershipTimeoutMs: 5_000,
    sseMembership: vi.fn<BrowserAsyncTransportOptions["sseMembership"]>(),
    timers: { clearTimeout: vi.fn(), timeout: vi.fn(() => 1) },
    webSocket() {
      const socket: FakeSocket = {
        close: vi.fn<(code?: number, reason?: string) => void>(),
        send: vi.fn<(data: string) => void>(),
      };
      sockets.push(socket);
      return socket;
    },
  };
  new BrowserAsyncTransportPorts(options).webSocket(connectRequest(failed));
  const socket = sockets[0];
  if (socket === undefined) throw new Error("socket_missing");
  socket.onopen?.();
  return socket;
}

describe("browser WebSocket adapter closure", () => {
  it("does not report a failure on the error event alone", () => {
    const failed = vi.fn();
    const socket = openSocket(failed);
    socket.onerror?.();
    expect(failed).not.toHaveBeenCalled();
  });

  it("reports exactly one failure from the close event that follows an error", () => {
    const failed = vi.fn();
    const socket = openSocket(failed);
    socket.onerror?.();
    socket.onclose?.({ code: 1008, reason: "invalid_envelope", wasClean: true });
    expect(failed).toHaveBeenCalledTimes(1);
    expect(failed).toHaveBeenCalledWith("transport_lost");
  });

  it("reports one failure from a close event with no preceding error", () => {
    const failed = vi.fn();
    const socket = openSocket(failed);
    socket.onclose?.({ code: 1006, reason: "", wasClean: false });
    expect(failed).toHaveBeenCalledTimes(1);
    expect(failed).toHaveBeenCalledWith("transport_lost");
  });
});
