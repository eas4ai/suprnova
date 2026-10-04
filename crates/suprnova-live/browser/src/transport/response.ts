import { parseCanonicalJson } from "../canonical.js";
import { breachOf, limitBreach, type LiveLimits } from "../limits.js";
import { validateUpdateResponse } from "../protocol.js";
import { asNumber, asRecord, asString } from "../schema.js";
import type { BuiltLiveRequest } from "./request.js";
import { LiveTransportError, type LiveTransportResponse } from "./state.js";

const ACCEPTED_STATUSES = new Set([200, 409, 422, 500]);

function tooLarge(measured: number, maximum: number, atLeast: boolean): LiveTransportError {
  return new LiveTransportError(
    "size",
    null,
    limitBreach("maxResponseBytes", measured, maximum, { atLeast }),
  );
}

/// Reads the body under the server's configured response limit, so a reply
/// larger than the server would send (a proxy's, say) cannot exhaust memory.
async function boundedBytes(response: Response, maximum: number): Promise<Uint8Array> {
  const declared = response.headers.get("content-length");
  if (declared !== null) {
    if (!/^(0|[1-9][0-9]*)$/u.test(declared)) throw new LiveTransportError("size");
    if (Number(declared) > maximum) throw tooLarge(Number(declared), maximum, false);
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
        throw tooLarge(total, maximum, true);
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
  limits: LiveLimits,
): Promise<LiveTransportResponse> {
  if (!ACCEPTED_STATUSES.has(response.status)) {
    throw new LiveTransportError("http", response.status);
  }
  if (response.headers.get("content-type") !== request.mediaType) {
    throw new LiveTransportError("media", response.status);
  }
  const bytes = await boundedBytes(response, limits.maxResponseBytes);
  let text: string;
  try {
    text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
    validateUpdateResponse(text, limits);
  } catch (error: unknown) {
    if (error instanceof LiveTransportError) throw error;
    throw new LiveTransportError("protocol", response.status, breachOf(error));
  }
  try {
    const root = asRecord(parseCanonicalJson(text, { maxDepth: limits.maxJsonDepth }));
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
