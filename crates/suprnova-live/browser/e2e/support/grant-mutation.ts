import { Buffer } from "node:buffer";

export function forgeCanonicalGrantSignature(grant: string): string {
  const parts = grant.split(".");
  if (parts.length !== 4) throw new Error("transfer_grant_shape_invalid");
  const signature = parts[3];
  if (signature === undefined || signature.length === 0) {
    throw new Error("transfer_grant_signature_missing");
  }
  const signatureBytes = Buffer.from(signature, "base64url");
  if (signatureBytes.length !== 32 || signatureBytes.toString("base64url") !== signature) {
    throw new Error("transfer_grant_signature_noncanonical");
  }
  signatureBytes[0] = (signatureBytes[0] ?? 0) ^ 0x01;
  const forgedSignature = signatureBytes.toString("base64url");
  if (forgedSignature.length !== signature.length) {
    throw new Error("transfer_grant_signature_length_changed");
  }
  parts[3] = forgedSignature;
  return parts.join(".");
}
