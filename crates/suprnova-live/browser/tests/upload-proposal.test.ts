import { describe, expect, it, vi } from "vitest";

import type { OwnedDirective } from "../src/directives/ownership.js";
import { DirectiveOwnership } from "../src/directives/ownership.js";
import { IslandRecord } from "../src/islands/record.js";
import type { IslandMetadata } from "../src/islands/metadata.js";
import { ModelFormRuntime } from "../src/models/forms.js";
import type { RuntimeClock, RuntimeScheduler } from "../src/runtime/ports.js";
import { declaresUploadField, UploadProposalAuthority } from "../src/uploads/proposal.js";

const FIRST = "018f47c1-2af0-7cc4-a001-000000000001";
const SECOND = "018f47c1-2af0-7cc4-a001-000000000002";

function proposalContext(
  overrides: Partial<{
    active: () => boolean;
    declared: (field: string) => boolean;
    write: (value: unknown) => boolean;
  }> = {},
) {
  return {
    active: overrides.active ?? (() => true),
    declared: overrides.declared ?? ((field: string) => field === "avatar"),
    write: overrides.write ?? (() => true),
  };
}

describe("core upload proposal authority", () => {
  it("finds only the exact bounded declaration outside nested islands and model conflicts", () => {
    function element(
      attributes: readonly Readonly<{ name: string; value: string }>[],
      children: Element[] = [],
      island = false,
    ): Element {
      const collection = Object.assign(children, {
        item(index: number) {
          return children[index] ?? null;
        },
      });
      return {
        attributes,
        children: collection,
        matches: () => island,
        shadowRoot: null,
      } as unknown as Element;
    }
    const declared = element([{ name: "live:upload", value: "avatar" }]);
    const nested = element([{ name: "live:upload", value: "nested" }], [], true);
    const conflict = element([
      { name: "live:upload", value: "conflicted" },
      { name: "live:model.blur", value: "conflicted" },
    ]);
    const separateConflict = element([{ name: "live:model.blur", value: "avatar" }]);
    const root = element([], [declared, nested, conflict, separateConflict], true);

    expect(declaresUploadField(root, "avatar")).toBe(false);
    expect(declaresUploadField(root, "nested")).toBe(false);
    expect(declaresUploadField(root, "conflicted")).toBe(false);
  });

  it("accepts canonical single and multiple handles and preserves unchanged outcomes", () => {
    const authority = new UploadProposalAuthority<object>();
    const owner = {};
    const write = vi.fn(() => true);
    expect(authority.propose(owner, "avatar", FIRST, proposalContext({ write }))).toBe("accepted");
    expect(authority.propose(owner, "avatar", [FIRST, SECOND], proposalContext({ write }))).toBe(
      "accepted",
    );
    expect(write).toHaveBeenLastCalledWith([FIRST, SECOND]);

    expect(
      authority.propose(owner, "avatar", [FIRST, SECOND], proposalContext({ write: () => false })),
    ).toBe("unchanged");
  });

  it("rejects undeclared, malformed, duplicate, cross-field, and cross-island proposals", () => {
    const authority = new UploadProposalAuthority<object>();
    const owner = {};
    expect(() => authority.propose(owner, "other", FIRST, proposalContext())).toThrow(
      "feature_upload_field_undeclared",
    );
    expect(() => authority.propose(owner, "avatar", "not-a-handle", proposalContext())).toThrow(
      "upload_handle_invalid",
    );
    expect(() => authority.propose(owner, "avatar", [FIRST, FIRST], proposalContext())).toThrow(
      "upload_handle_proposal_invalid",
    );
    authority.propose(owner, "avatar", FIRST, proposalContext());
    expect(() =>
      authority.propose(owner, "other", FIRST, proposalContext({ declared: () => true })),
    ).toThrow("feature_upload_handle_scope_invalid");
    expect(() => authority.propose({}, "avatar", FIRST, proposalContext())).toThrow(
      "feature_upload_handle_scope_invalid",
    );
  });

  it("returns retired before reading an untrusted proposal and bounds claims", () => {
    const authority = new UploadProposalAuthority<object>(1);
    expect(
      authority.propose(
        {},
        "not read",
        "not read",
        proposalContext({ active: () => false, declared: () => false }),
      ),
    ).toBe("retired");
    authority.propose({}, "avatar", FIRST, proposalContext());
    expect(() => authority.propose({}, "avatar", SECOND, proposalContext())).toThrow(
      "feature_upload_handle_limit",
    );
  });
});

describe("typed upload model batching", () => {
  it("places the opaque handle, and only the handle, in the next deliberate action batch", () => {
    const element = { setAttribute: vi.fn() } as unknown as Element;
    const record = new IslandRecord(element, Object.create(null) as IslandMetadata);
    const clock: RuntimeClock = { now: () => 0 };
    const scheduler: RuntimeScheduler = {
      animationFrame: () => 1,
      cancelAnimationFrame: () => undefined,
      clearTimeout: () => undefined,
      microtask: (callback) => {
        callback();
      },
      timeout: () => 1,
    };
    const forms = new ModelFormRuntime(new DirectiveOwnership(), clock, scheduler, () => null);
    forms.connect(record, []);
    forms.proposeTyped(record, "avatar", FIRST);
    const owned: OwnedDirective = {
      attributeName: "live:click",
      directive: { modifiers: [], name: "click", ok: true, value: "save" },
      element,
      island: record,
    };

    const batch = forms.prepareAction(owned, "click");
    expect(batch).toEqual({
      editSequences: { avatar: 1n },
      operations: [{ field: "avatar", kind: "sync_model" }],
      proposals: { avatar: FIRST },
    });
    expect(JSON.stringify(batch.proposals)).not.toContain("grant");
    expect(forms.prepareAction(owned, "init")).toEqual({
      editSequences: {},
      operations: [],
      proposals: {},
    });
    record.dispose();
  });
});
