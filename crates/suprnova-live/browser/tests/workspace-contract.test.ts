import { describe, expect, it } from "vitest";

import {
  ENGINE_VERSION,
  RUNTIME_CONTRACT_VERSION,
  SUPPORTED_PROTOCOL_VERSIONS,
  SUPPORTED_SNAPSHOT_VERSIONS,
} from "../src/index.js";

describe("browser workspace contract", () => {
  it("exposes independent runtime, snapshot, and rolling protocol versions", () => {
    expect(ENGINE_VERSION).toBe("0.1.0");
    expect(RUNTIME_CONTRACT_VERSION).toBe(1);
    expect(SUPPORTED_SNAPSHOT_VERSIONS).toEqual([1]);
    expect(SUPPORTED_PROTOCOL_VERSIONS).toEqual([1, 2]);
  });
});
