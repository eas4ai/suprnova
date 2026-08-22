import { describe, expect, it } from "vitest";

import {
  ENGINE_VERSION,
  SUPPORTED_PROTOCOL_VERSIONS,
  SUPPORTED_SNAPSHOT_VERSIONS,
} from "../src/index.js";

describe("iteration 002 workspace contract", () => {
  it("exposes the engine, v1 snapshot, and rolling v1/v2 protocol contracts", () => {
    expect(ENGINE_VERSION).toBe("0.1.0");
    expect(SUPPORTED_SNAPSHOT_VERSIONS).toEqual([1]);
    expect(SUPPORTED_PROTOCOL_VERSIONS).toEqual([1, 2]);
  });
});
