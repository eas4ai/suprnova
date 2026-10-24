import type { RuntimeRandomness } from "../runtime/ports.js";

export const PROMOTION_NONCE_BYTES = 16;

function base64url(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return globalThis.btoa(binary).replace(/\+/gu, "-").replace(/\//gu, "_").replace(/=+$/u, "");
}

export function createPromotionNonce(randomness: RuntimeRandomness): string {
  try {
    const bytes = randomness.randomBytes(PROMOTION_NONCE_BYTES);
    if (!(bytes instanceof Uint8Array) || bytes.byteLength !== PROMOTION_NONCE_BYTES) {
      throw new Error("invalid_randomness_length");
    }
    return base64url(bytes);
  } catch {
    throw new Error("promotion_nonce_unavailable");
  }
}
