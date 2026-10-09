import { describe, expect, it } from "vitest";

import {
  NavigationEligibilityError,
  nativeNavigationIntent,
  type NativeNavigationCandidate,
} from "../src/navigation/eligibility.js";

const current = new URL("https://app.example.test/catalog?page=1");

function candidate(overrides: Partial<NativeNavigationCandidate> = {}): NativeNavigationCandidate {
  return {
    base: current,
    history: "navigate",
    method: "GET",
    prefetch: "none",
    source: "anchor",
    target: "/products?page=2",
    transitionName: null,
    ...overrides,
  };
}

describe("native document navigation eligibility", () => {
  it("describes ordinary anchors, GET/POST forms, redirects, and refresh without route state", () => {
    expect(nativeNavigationIntent(candidate())).toMatchObject({
      history: "navigate",
      method: "GET",
      prefetch: "none",
      transitionName: null,
    });
    expect(nativeNavigationIntent(candidate({ method: "POST", source: "form" })).method).toBe(
      "POST",
    );
    expect(nativeNavigationIntent(candidate({ source: "redirect" })).target.pathname).toBe(
      "/products",
    );
    expect(
      nativeNavigationIntent(
        candidate({ source: "refresh", target: "https://app.example.test/catalog?page=1" }),
      ).target.href,
    ).toBe(current.href);
  });

  it("leaves fragments, downloads, external origins, targets, and modified activation native-only", () => {
    const cases: readonly Partial<NativeNavigationCandidate>[] = [
      { target: "#details", prefetch: "link", transitionName: "hero" },
      { download: true, prefetch: "link", transitionName: "hero" },
      {
        target: "https://docs.example.test/guide",
        prefetch: "link",
        transitionName: "hero",
      },
      { targetContext: "_blank", prefetch: "link", transitionName: "hero" },
      {
        activation: { altKey: false, button: 0, ctrlKey: true, metaKey: false, shiftKey: false },
        prefetch: "link",
        transitionName: "hero",
      },
    ];

    for (const overrides of cases) {
      expect(nativeNavigationIntent(candidate(overrides))).toMatchObject({
        history: "navigate",
        prefetch: "none",
        transitionName: null,
      });
    }
  });

  it("keeps content negotiation and error documents on ordinary navigation fallback", () => {
    expect(
      nativeNavigationIntent(
        candidate({
          response: { mediaType: "application/pdf", status: 200 },
          transitionName: "document",
        }),
      ).transitionName,
    ).toBeNull();
    expect(
      nativeNavigationIntent(
        candidate({
          response: { mediaType: "text/html", status: 404 },
          transitionName: "document",
        }),
      ).transitionName,
    ).toBeNull();
  });

  it("accepts checked same-origin enhancements and rejects unsafe transition names", () => {
    expect(
      nativeNavigationIntent(
        candidate({ prefetch: "speculation", transitionName: "catalog-hero" }),
      ),
    ).toMatchObject({ prefetch: "speculation", transitionName: "catalog-hero" });
    for (const transitionName of ["none", "two words", "x".repeat(65), "--reserved"]) {
      expect(() => nativeNavigationIntent(candidate({ transitionName }))).toThrow(
        NavigationEligibilityError,
      );
    }
    expect(
      nativeNavigationIntent(
        candidate({ target: "/unsafe\\path", prefetch: "link", transitionName: "hero" }),
      ),
    ).toMatchObject({ prefetch: "none", transitionName: null });
  });

  it("permits replaceState only for validated same-route reflection", () => {
    expect(
      nativeNavigationIntent(
        candidate({
          history: "replace_query",
          source: "reflection",
          target: "/catalog?page=2#results",
        }),
      ),
    ).toMatchObject({ history: "replace_query", method: "GET" });

    for (const overrides of [
      { history: "replace_query" as const, source: "anchor" as const },
      {
        history: "replace_query" as const,
        source: "reflection" as const,
        target: "/other?page=2",
      },
      {
        history: "replace_query" as const,
        source: "reflection" as const,
        target: "https://evil.example/catalog?page=2",
      },
    ]) {
      expect(() => nativeNavigationIntent(candidate(overrides))).toThrow(
        NavigationEligibilityError,
      );
    }
  });

  it("rejects unsupported methods, credentials, schemes, and malformed targets", () => {
    for (const overrides of [
      { method: "PUT" as "GET" },
      { target: "javascript:alert(1)" },
      { target: "https://user:secret@app.example.test/products" },
      { target: "https://[invalid" },
    ]) {
      expect(() => nativeNavigationIntent(candidate(overrides))).toThrow(
        NavigationEligibilityError,
      );
    }
  });
});
