import { parseCanonicalJson } from "../canonical.js";
import { validateUpdateResponse } from "../protocol.js";
import { asNumber, asRecord, asString } from "../schema.js";
import type { BuiltLiveRequest } from "./request.js";
import { LiveTransportError, type LiveTransportResponse } from "./state.js";

const ACCEPTED_STATUSES = new Set([200, 409, 422, 500]);

async function boundedBytes(response: Response, maximum: number): Promise<Uint8Array> {
  const declared = response.headers.get("content-length");
  if (declared !== null) {
    if (!/^(0|[1-9][0-9]*)$/u.test(declared) || Number(declared) > maximum) {
      throw new LiveTransportError("size");
    }
  }
  if (response.body === null) return new Uint8Array();
  const reader = response.body.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  try {
    let chunk = await reader.read();
    while (!chunk.done) {
      total += chunk.value.byteLength;
      if (total > maximum) {
        void reader.cancel().catch(() => undefined);
        throw new LiveTransportError("size");
      }
      chunks.push(chunk.value);
      chunk = await reader.read();
    }
  } finally {
    reader.releaseLock();
  }
  const bytes = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return bytes;
}

export async function readLiveResponse(
  request: BuiltLiveRequest,
  response: Response,
  maximumBytes: number,
): Promise<LiveTransportResponse> {
  if (!ACCEPTED_STATUSES.has(response.status)) {
    throw new LiveTransportError("http", response.status);
  }
  if (response.headers.get("content-type") !== request.mediaType) {
    throw new LiveTransportError("media", response.status);
  }
  const bytes = await boundedBytes(response, maximumBytes);
  let text: string;
  try {
    text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
    validateUpdateResponse(text);
  } catch (error: unknown) {
    if (error instanceof LiveTransportError) throw error;
    throw new LiveTransportError("protocol", response.status);
  }
  try {
    const root = asRecord(parseCanonicalJson(text));
    if (asNumber(root["protocol_version"]) !== request.protocolVersion) {
      throw new LiveTransportError("protocol", response.status);
    }
    if (asString(root["correlation_id"]) !== request.identity.correlationId) {
      throw new LiveTransportError("correlation", response.status);
    }
  } catch (error: unknown) {
    if (error instanceof LiveTransportError) throw error;
    throw new LiveTransportError("protocol", response.status);
  }
  return Object.freeze({ protocolVersion: request.protocolVersion, status: response.status, text });
}
