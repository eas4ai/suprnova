import { canonicalize, type JsonValue } from "./canonical.js";
import { asJsonValue, asRecord, asString } from "./schema.js";

const encoder = new TextEncoder();
const salt = encoder.encode("suprnova-live/snapshot-hkdf/v1");

function arrayBuffer(bytes: Uint8Array): ArrayBuffer {
  const buffer = new ArrayBuffer(bytes.byteLength);
  new Uint8Array(buffer).set(bytes);
  return buffer;
}

function hexBytes(hex: string): Uint8Array {
  if (!/^(?:[0-9a-f]{2})+$/u.test(hex)) throw new TypeError("invalid_hex");
  const bytes = new Uint8Array(hex.length / 2);
  for (let index = 0; index < bytes.length; index += 1) {
    const pair = hex.slice(index * 2, index * 2 + 2);
    const value = Number.parseInt(pair, 16);
    bytes[index] = value;
  }
  return bytes;
}

function base64urlBytes(value: string): Uint8Array {
  if (!/^[A-Za-z0-9_-]+$/u.test(value) || value.includes("=")) {
    throw new TypeError("invalid_base64url");
  }
  const base64 = value.replace(/-/gu, "+").replace(/_/gu, "/");
  const padded = base64.padEnd(Math.ceil(base64.length / 4) * 4, "=");
  const binary = atob(padded);
  const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
  if (base64url(bytes) !== value) throw new TypeError("noncanonical_base64url");
  return bytes;
}

function base64url(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/gu, "-").replace(/\//gu, "_").replace(/=+$/u, "");
}

async function signingKey(rootHex: string, purpose: "seed" | "instance"): Promise<CryptoKey> {
  const root = await crypto.subtle.importKey("raw", arrayBuffer(hexBytes(rootHex)), "HKDF", false, [
    "deriveKey",
  ]);
  const info = encoder.encode(
    purpose === "seed" ? "suprnova-live/seed-signature/v1" : "suprnova-live/instance-signature/v1",
  );
  return crypto.subtle.deriveKey(
    { name: "HKDF", hash: "SHA-256", salt: arrayBuffer(salt), info: arrayBuffer(info) },
    root,
    { name: "HMAC", hash: "SHA-256", length: 256 },
    false,
    ["verify"],
  );
}

export interface SnapshotVerification {
  readonly ok: boolean;
  readonly error?: string;
}

export async function verifySnapshotFixture(
  encoded: JsonValue,
  rootHex: string,
  purpose: "seed" | "instance",
  now: number,
): Promise<SnapshotVerification> {
  const envelope = asRecord(encoded);
  const bodyUnknown = envelope["body"];
  const signature = asString(envelope["signature"]);
  if (bodyUnknown === null || typeof bodyUnknown !== "object" || Array.isArray(bodyUnknown)) {
    return { ok: false, error: "invalid_envelope" };
  }
  const body: JsonValue = asJsonValue(bodyUnknown);
  let signatureBytes: Uint8Array;
  try {
    signatureBytes = base64urlBytes(signature);
  } catch {
    return { ok: false, error: "signature_invalid" };
  }
  if (signatureBytes.byteLength !== 32) return { ok: false, error: "signature_invalid" };
  const key = await signingKey(rootHex, purpose);
  const valid = await crypto.subtle.verify(
    "HMAC",
    key,
    arrayBuffer(signatureBytes),
    arrayBuffer(encoder.encode(canonicalize(body))),
  );
  if (!valid) return { ok: false, error: "signature_invalid" };
  const fields = asRecord(body);
  if (asString(fields["form"]) !== purpose) return { ok: false, error: "wrong_form" };
  const issued = Number(asString(fields["issued_at"]));
  if (purpose === "seed") {
    const maxAge = Number(asString(fields["max_age_ms"]));
    if (now > issued + maxAge + 50) return { ok: false, error: "expired" };
  } else if (now >= Number(asString(fields["expires_at"]))) {
    return { ok: false, error: "expired" };
  }
  return { ok: true };
}
