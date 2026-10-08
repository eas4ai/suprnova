import { describe, expect, it } from "vitest";

import { nativeNavigationIntent } from "../src/navigation/eligibility.js";
import {
  PrefetchCoordinator,
  prefetchEligibility,
  type PrefetchContext,
  type PrefetchEmission,
  type PrefetchHost,
} from "../src/navigation/prefetch.js";

const current = new URL("https://app.example.test/catalog");

function intent(
  method: "GET" | "HEAD" | "POST" = "GET",
  prefetch: "link" | "speculation" = "link",
) {
  return nativeNavigationIntent({
    base: current,
    history: "navigate",
    method,
    prefetch,
    source: method === "POST" ? "form" : "anchor",
    target: "/products",
    transitionName: null,
  });
}

function context(overrides: Partial<PrefetchContext> = {}): PrefetchContext {
  return {
    cachePolicy: "public",
    consumesFlash: false,
    current,
    explicit: true,
    hidden: false,
    redirectProne: false,
    saveData: false,
    variesBy: [],
    ...overrides,
  };
}

class RecordingHost implements PrefetchHost {
  readonly emissions: { readonly kind: string; readonly href: string; canceled: boolean }[] = [];

  emit(kind: "link" | "speculation", target: URL): PrefetchEmission {
    const recorded = { kind, href: target.href, canceled: false };
    this.emissions.push(recorded);
    return {
      cancel() {
        recorded.canceled = true;
      },
    };
  }
}

describe("privacy-aware native prefetch", () => {
  it("accepts only explicit same-origin public GET/HEAD targets", () => {
    expect(prefetchEligibility(intent("GET"), context())).toEqual({ eligible: true });
    expect(prefetchEligibility(intent("HEAD"), context())).toEqual({ eligible: true });
  });

  it("does not emit when the navigation intent did not request prefetch", () => {
    const disabled = { ...intent(), prefetch: "none" as const };
    expect(prefetchEligibility(disabled, context())).toEqual({
      eligible: false,
      reason: "not_explicit",
    });
  });

  it.each([
    ["method", intent("POST"), context()],
    ["cross_origin", { ...intent(), target: new URL("https://other.example/products") }, context()],
    ["credentials_variance", intent(), context({ variesBy: ["credentials"] })],
    ["tenant_variance", intent(), context({ variesBy: ["tenant"] })],
    ["principal_variance", intent(), context({ variesBy: ["principal"] })],
    ["locale_variance", intent(), context({ variesBy: ["locale"] })],
    ["flash", intent(), context({ consumesFlash: true })],
    ["no_store", intent(), context({ cachePolicy: "no-store" })],
    ["private", intent(), context({ cachePolicy: "private" })],
    ["data_saver", intent(), context({ saveData: true })],
    ["redirect", intent(), context({ redirectProne: true })],
    ["hidden", intent(), context({ hidden: true })],
    ["not_explicit", intent(), context({ explicit: false })],
  ] as const)("rejects %s before emitting a browser resource", (reason, candidate, metadata) => {
    expect(prefetchEligibility(candidate, metadata)).toEqual({ eligible: false, reason });
  });

  it("emits bounded native resources without exposing a document-body fetch port", () => {
    const host = new RecordingHost();
    const coordinator = new PrefetchCoordinator({ host, maxConcurrent: 2 });

    expect(coordinator.request("one", intent(), context())).toBe("emitted");
    expect(
      coordinator.request(
        "two",
        { ...intent("GET", "speculation"), target: new URL("/two", current) },
        context(),
      ),
    ).toBe("emitted");
    expect(
      coordinator.request("three", { ...intent(), target: new URL("/three", current) }, context()),
    ).toBe("capacity");
    expect(host.emissions).toEqual([
      { canceled: false, href: "https://app.example.test/products", kind: "link" },
      { canceled: false, href: "https://app.example.test/two", kind: "speculation" },
    ]);
    expect("fetch" in host).toBe(false);
  });

  it("deduplicates, cancels on leave/removal, and releases capacity", () => {
    const host = new RecordingHost();
    const coordinator = new PrefetchCoordinator({ host, maxConcurrent: 1 });

    expect(coordinator.request("one", intent(), context())).toBe("emitted");
    expect(coordinator.request("one", intent(), context())).toBe("duplicate");
    expect(coordinator.request("same-url", intent(), context())).toBe("duplicate");
    expect(coordinator.cancel("one")).toBe(true);
    expect(host.emissions[0]?.canceled).toBe(true);
    expect(coordinator.cancel("one")).toBe(false);
    expect(coordinator.request("two", intent(), context())).toBe("emitted");
    coordinator.removed("two");
    expect(host.emissions[1]?.canceled).toBe(true);
  });

  it("never emits an ineligible target", () => {
    const host = new RecordingHost();
    const coordinator = new PrefetchCoordinator({ host, maxConcurrent: 2 });

    expect(coordinator.request("private", intent(), context({ cachePolicy: "private" }))).toBe(
      "ineligible",
    );
    expect(host.emissions).toEqual([]);
  });
});
