import type { JsonValue } from "../canonical.js";
import { ISLAND_ROOT_SELECTOR } from "../islands/metadata.js";
import {
  validateUploadField,
  validateUploadProposal,
  type UploadHandle,
  type UploadHandleProposal,
  type UploadHandleProposalDisposition,
} from "./types.js";

interface UploadHandleClaim<Owner extends object> {
  readonly field: string;
  readonly owner: Owner;
}

export interface UploadProposalContext {
  active(): boolean;
  declared(field: string): boolean;
  write(value: JsonValue): boolean;
}

const MAX_DECLARATION_ELEMENTS = 4_096;
const MAX_DECLARATION_ATTRIBUTES = 64;

export function declaresUploadField(root: Element, field: string): boolean {
  validateUploadField(field);
  const pending: Element[] = [root];
  let scanned = 0;
  let declared = false;
  let modelConflict = false;
  try {
    while (pending.length > 0) {
      const element = pending.pop();
      if (element === undefined) break;
      if (element !== root && element.matches(ISLAND_ROOT_SELECTOR)) continue;
      scanned += 1;
      if (
        scanned > MAX_DECLARATION_ELEMENTS ||
        element.attributes.length > MAX_DECLARATION_ATTRIBUTES
      ) {
        return false;
      }
      for (const attribute of element.attributes) {
        if (attribute.name === "live:upload" && attribute.value === field) declared = true;
        if (
          attribute.value === field &&
          (attribute.name === "live:model" || attribute.name.startsWith("live:model."))
        ) {
          modelConflict = true;
        }
      }
      const shadow = "shadowRoot" in element ? element.shadowRoot : null;
      if (shadow !== null) {
        for (let index = shadow.children.length - 1; index >= 0; index -= 1) {
          const child = shadow.children.item(index);
          if (child !== null) pending.push(child);
        }
      }
      for (let index = element.children.length - 1; index >= 0; index -= 1) {
        const child = element.children.item(index);
        if (child !== null) pending.push(child);
      }
    }
  } catch {
    return false;
  }
  return declared && !modelConflict;
}

export class UploadProposalAuthority<Owner extends object> {
  readonly #claims = new Map<UploadHandle, UploadHandleClaim<Owner>>();
  readonly #maximumClaims: number;

  constructor(maximumClaims = 4_096) {
    if (!Number.isSafeInteger(maximumClaims) || maximumClaims < 1 || maximumClaims > 65_536) {
      throw new RangeError("feature_upload_handle_limit_invalid");
    }
    this.#maximumClaims = maximumClaims;
  }

  propose(
    owner: Owner,
    field: string,
    proposal: UploadHandleProposal,
    context: UploadProposalContext,
  ): UploadHandleProposalDisposition {
    if (!context.active()) return "retired";
    validateUploadField(field);
    validateUploadProposal(proposal);
    if (!context.declared(field)) throw new Error("feature_upload_field_undeclared");
    const handles = proposal === null ? [] : typeof proposal === "string" ? [proposal] : proposal;
    let newClaims = 0;
    for (const handle of handles) {
      const claim = this.#claims.get(handle);
      if (claim === undefined) newClaims += 1;
      else if (claim.owner !== owner || claim.field !== field) {
        throw new Error("feature_upload_handle_scope_invalid");
      }
    }
    if (newClaims > this.#maximumClaims - this.#claims.size) {
      throw new Error("feature_upload_handle_limit");
    }
    for (const handle of handles) {
      if (!this.#claims.has(handle)) this.#claims.set(handle, Object.freeze({ field, owner }));
    }
    const value: JsonValue =
      proposal === null || typeof proposal === "string" ? proposal : Object.freeze([...proposal]);
    return context.write(value) ? "accepted" : "unchanged";
  }

  dispose(): void {
    this.#claims.clear();
  }
}
