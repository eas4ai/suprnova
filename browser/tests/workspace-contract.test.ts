import { describe, expect, it } from "vitest";

import {
  ENGINE_VERSION,
  SUPPORTED_PROTOCOL_VERSIONS,
  SUPPORTED_SNAPSHOT_VERSIONS,
} from "../src/index.js";

describe("iteration 001 workspace contract", () => {
  it("exposes the engine and v1 protocol contracts", () => {
    expect(ENGINE_VERSION).toBe("0.1.0");
    expect(SUPPORTED_SNAPSHOT_VERSIONS).toEqual([1]);
    expect(SUPPORTED_PROTOCOL_VERSIONS).toEqual([1]);
  });
});
