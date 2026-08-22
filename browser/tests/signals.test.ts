import { describe, expect, it } from "vitest";

import { parseDirective } from "../src/directives/parser.js";
import type { RuntimeScheduler } from "../src/runtime/ports.js";
import { SignalGraph } from "../src/signals/graph.js";
import { LocalSignalScope } from "../src/signals/scope.js";
import { parseSignalDeclarations, parseSignalLiteral } from "../src/signals/value.js";

describe("typed local signal values", () => {
  it("accepts only bounded boolean, string, safe integer, and null literals", () => {
    expect(parseSignalLiteral("true")).toBe(true);
    expect(parseSignalLiteral("false")).toBe(false);
    expect(parseSignalLiteral("null")).toBeNull();
    expect(parseSignalLiteral("-42")).toBe(-42);
    expect(parseSignalLiteral("hello-world")).toBe("hello-world");
    for (const invalid of ["", "01", "1.5", "{}", "window.alert", "9007199254740992"]) {
      expect(() => parseSignalLiteral(invalid)).toThrow("signal_literal_invalid");
    }
  });

  it("rejects duplicate declarations and excessive mappings", () => {
    expect(() => parseSignalDeclarations("open:false,open:true")).toThrow(
      "signal_declaration_duplicate",
    );
    expect(() =>
      parseSignalDeclarations(
        Array.from({ length: 33 }, (_, index) => `signal${String(index)}:false`).join(","),
      ),
    ).toThrow("signal_declaration_limit");
  });

  it("keeps signal integers inside the browser's exact safe range", () => {
    expect(parseDirective("live:signal", "count:9007199254740991")).toMatchObject({
      ok: true,
    });
    expect(parseDirective("live:signal", "count:9007199254740992")).toMatchObject({
      ok: false,
      code: "invalid_value",
    });
  });
});

describe("lexical signal scopes", () => {
  it("uses deterministic shadowing without leaking or mutating an ancestor", () => {
    const parent = new LocalSignalScope("parent", parseSignalDeclarations("open:false"));
    const child = new LocalSignalScope("child", parseSignalDeclarations("open:true"), parent);
    expect(parent.get("open")).toBe(false);
    expect(child.get("open")).toBe(true);
    child.toggle("open");
    expect(child.get("open")).toBe(false);
    expect(parent.get("open")).toBe(false);
    expect(() => child.get("missing")).toThrow("signal_missing");
  });

  it("batches changes, suppresses same values, resets, and disposes exactly once", () => {
    const changes: string[][] = [];
    const scope = new LocalSignalScope(
      "root",
      parseSignalDeclarations("open:false,label:hello"),
      null,
      (_scope, names) => {
        changes.push([...names]);
      },
    );
    scope.batch(() => {
      scope.set("open", true);
      scope.set("open", true);
      scope.set("label", "world");
    });
    expect(changes).toEqual([["open", "label"]]);
    scope.reset("open");
    expect(scope.get("open")).toBe(false);
    scope.dispose();
    scope.dispose();
    expect(() => {
      scope.set("open", true);
    }).toThrow("signal_scope_disposed");
  });

  it("restores an exact compatible capture atomically", () => {
    const scope = new LocalSignalScope(
      "restored",
      parseSignalDeclarations("open:false,label:hello"),
    );
    expect(scope.restore({ open: true, label: "world" })).toBe(true);
    expect(scope.values()).toEqual({ open: true, label: "world" });
    expect(scope.restore({ open: false, label: 4 })).toBe(false);
    expect(scope.values()).toEqual({ open: true, label: "world" });
    expect(scope.restore({ open: false })).toBe(false);
    expect(scope.values()).toEqual({ open: true, label: "world" });
  });

  it("treats identifier literals as values rather than expression references", () => {
    const scope = new LocalSignalScope(
      "acyclic",
      parseSignalDeclarations("first:second,second:first"),
    );
    expect(scope.get("first")).toBe("second");
    expect(scope.get("second")).toBe("first");
  });
});

describe("signal dependency graph", () => {
  it("does not retain a target whose initial projection fails", () => {
    const scheduler: RuntimeScheduler = {
      microtask(callback) {
        callback();
      },
      animationFrame() {
        return 1;
      },
      cancelAnimationFrame() {
        return undefined;
      },
      timeout() {
        return 1;
      },
      clearTimeout() {
        return undefined;
      },
    };
    const scope = new LocalSignalScope("graph", parseSignalDeclarations("open:false"));
    const graph = new SignalGraph(scheduler);
    let disposals = 0;
    const target = {
      element: Object.create(null) as Element,
      apply() {
        throw new Error("projection_failed");
      },
      dispose() {
        disposals += 1;
      },
    };

    expect(() => graph.register(scope, "open", target)).toThrow("projection_failed");
    target.dispose();
    graph.dispose();
    expect(disposals).toBe(1);
  });
});
