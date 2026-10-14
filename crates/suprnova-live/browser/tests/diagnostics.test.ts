import fc from "fast-check";
import { describe, expect, it } from "vitest";

import { CoreRuntimeDiagnostics, RuntimeDiagnostics } from "../src/runtime/diagnostics.js";

const CONFIGURATION_FAILURE = {
  code: "configuration_invalid",
  severity: "error",
  phase: "configuration",
  detailCode: "invalid_shape",
} as const;

describe("redacted runtime diagnostics", () => {
  it("keeps core diagnostic work active without retaining the optional diagnostic ledger", () => {
    const diagnostics = new CoreRuntimeDiagnostics("errors");

    expect(() => {
      diagnostics.record({
        code: "lifecycle_notice",
        severity: "info",
        phase: "lifecycle",
        detailCode: "connected",
      });
    }).not.toThrow();
    expect(() => {
      diagnostics.record(CONFIGURATION_FAILURE);
    }).not.toThrow();
    expect(() => {
      diagnostics.record(CONFIGURATION_FAILURE);
    }).not.toThrow();
    expect("entries" in diagnostics).toBe(false);
    expect(() => new CoreRuntimeDiagnostics("document-verbose")).toThrow("runtime_diagnostic_mode");
  });

  it("applies off, errors, and verbose modes without accepting document authority", () => {
    const off = new RuntimeDiagnostics({ mode: "off" });
    expect(off.record(CONFIGURATION_FAILURE)).toBeNull();

    const errors = new RuntimeDiagnostics({ mode: "errors" });
    expect(
      errors.record({
        code: "lifecycle_notice",
        severity: "info",
        phase: "lifecycle",
        detailCode: "connected",
      }),
    ).toBeNull();
    expect(errors.record(CONFIGURATION_FAILURE)).toMatchObject({ sequence: 0 });

    const verbose = new RuntimeDiagnostics({ mode: "verbose" });
    expect(
      verbose.record({
        code: "lifecycle_notice",
        severity: "info",
        phase: "lifecycle",
        detailCode: "connected",
      }),
    ).toMatchObject({ sequence: 0 });
  });

  it("stores only closed fields and never retains unsafe diagnostic context", () => {
    const diagnostics = new RuntimeDiagnostics({ mode: "verbose" });
    const unsafe = {
      snapshot: "signed-secret",
      signature: "signature-secret",
      cookie: "session=secret",
      token: "bearer-secret",
      model: "private-model-value",
      html: "<script>secret</script>",
      url: "https://secret.example/path",
      instance: "instance-secret",
      correlation: "correlation-secret",
      idempotency: "idempotency-secret",
      error: new Error("exception-secret"),
    };

    diagnostics.record(CONFIGURATION_FAILURE, unsafe);
    const serialized = JSON.stringify(diagnostics.entries());
    expect(serialized).toBe(
      '[{"code":"configuration_invalid","severity":"error","phase":"configuration","detailCode":"invalid_shape","sequence":0}]',
    );
    for (const secret of Object.values(unsafe)) {
      if (typeof secret === "string") expect(serialized).not.toContain(secret);
    }
    expect(serialized).not.toContain("exception-secret");
  });

  it("bounds entries and monotonic sequence without wrapping", () => {
    const diagnostics = new RuntimeDiagnostics({
      mode: "verbose",
      maxEntries: 2,
      initialSequence: 4_294_967_294,
    });

    expect(diagnostics.record(CONFIGURATION_FAILURE)).toMatchObject({
      sequence: 4_294_967_294,
    });
    expect(diagnostics.record(CONFIGURATION_FAILURE)).toMatchObject({
      sequence: 4_294_967_295,
    });
    expect(diagnostics.record(CONFIGURATION_FAILURE)).toBeNull();
    expect(diagnostics.entries()).toHaveLength(2);
  });

  it("rejects forged modes and contains observer failures", () => {
    expect(() => new RuntimeDiagnostics({ mode: "document-verbose" as "verbose" })).toThrow(
      "runtime_diagnostic_mode",
    );

    const diagnostics = new RuntimeDiagnostics({
      mode: "verbose",
      emit() {
        throw new Error("observer-secret");
      },
    });
    expect(() => diagnostics.record(CONFIGURATION_FAILURE)).not.toThrow();
    expect(diagnostics.entries()).toHaveLength(1);
  });

  it("rejects runtime-forged categories and never echoes arbitrary strings", () => {
    const diagnostics = new RuntimeDiagnostics({ mode: "verbose" });
    fc.assert(
      fc.property(fc.string(), (secret) => {
        const marker = `unsafe-context:${secret}:end`;
        const result = diagnostics.record(
          {
            ...CONFIGURATION_FAILURE,
            code: marker,
          } as typeof CONFIGURATION_FAILURE,
          marker,
        );
        expect(result).toBeNull();
        expect(JSON.stringify(diagnostics.entries())).not.toContain(marker);
      }),
      { numRuns: 100 },
    );
  });
});
