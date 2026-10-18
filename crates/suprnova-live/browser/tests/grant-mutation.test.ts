import { Buffer } from "node:buffer";

import { describe, expect, it } from "vitest";

import { forgeCanonicalGrantSignature } from "../e2e/support/grant-mutation.js";

describe("canonical forged transfer grants", () => {
  it("preserves canonical base64url for every legal trailing sextet", () => {
    for (const trailingBits of [0, 1, 2, 3]) {
      const signature = Buffer.alloc(32);
      signature[signature.length - 1] = trailingBits;
      const encoded = signature.toString("base64url");
      const grant = `v1.upload-key.body.${encoded}`;
      const forged = forgeCanonicalGrantSignature(grant);
      const forgedSignature = forged.split(".")[3];
      expect(forgedSignature).toBeDefined();
      expect(forgedSignature).not.toBe(encoded);
      expect(forgedSignature).toHaveLength(encoded.length);
      expect(Buffer.from(forgedSignature ?? "", "base64url").toString("base64url")).toBe(
        forgedSignature,
      );
      expect(Buffer.from(forgedSignature ?? "", "base64url")).toHaveLength(32);
    }
  });
});
