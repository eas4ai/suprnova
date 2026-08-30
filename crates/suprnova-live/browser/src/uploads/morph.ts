import type { RuntimeFeatureDirectiveOwnership } from "../features/contract.js";
import { MAX_UPLOAD_FILES_PER_DOCUMENT, validateUploadField } from "./types.js";

const KEY_ATTRIBUTE = "data-suprnova-live-key";
const SAFE_KEY = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/u;

interface UploadMorphElement {
  readonly element: Element;
  readonly key: string;
  readonly token: string;
}

interface UploadMorphField {
  readonly elements: readonly UploadMorphElement[];
  readonly field: string;
  readonly valid: boolean;
}

export interface UploadMorphContinuity {
  readonly fields: readonly UploadMorphField[];
}

function elementRole(ownership: RuntimeFeatureDirectiveOwnership): string | null {
  const { directive, element } = ownership;
  if (directive.name === "progress" && directive.role === null) return "progress";
  if (directive.name !== "upload") return null;
  if (directive.role === null) {
    try {
      return element.tagName.toUpperCase() === "INPUT" &&
        (element as HTMLInputElement).type.toLowerCase() === "file"
        ? "input"
        : null;
    } catch {
      return null;
    }
  }
  if (directive.role !== "cancel" && directive.role !== "remove" && directive.role !== "retry") {
    return null;
  }
  try {
    return element.tagName.toUpperCase() === "BUTTON" ? directive.role : null;
  } catch {
    return null;
  }
}

function keyedElement(
  ownership: RuntimeFeatureDirectiveOwnership,
  role: string,
): UploadMorphElement | null {
  let key: string | null;
  try {
    key = ownership.element.getAttribute(KEY_ATTRIBUTE);
  } catch {
    return null;
  }
  if (key === null || !SAFE_KEY.test(key)) return null;
  return Object.freeze({
    element: ownership.element,
    key,
    token: `${role}:${key}`,
  });
}

function describeField(
  ownerships: readonly RuntimeFeatureDirectiveOwnership[],
  field: string,
): UploadMorphField {
  const elements: UploadMorphElement[] = [];
  const tokens = new Set<string>();
  let inputCount = 0;
  let valid = true;
  for (const ownership of ownerships) {
    if (ownership.directive.value !== field) continue;
    const role = elementRole(ownership);
    if (role === null) continue;
    if (role === "input") inputCount += 1;
    const item = keyedElement(ownership, role);
    if (item === null || tokens.has(item.token)) {
      valid = false;
      continue;
    }
    tokens.add(item.token);
    elements.push(item);
  }
  if (inputCount !== 1) valid = false;
  elements.sort((left, right) => left.token.localeCompare(right.token));
  return Object.freeze({ elements: Object.freeze(elements), field, valid });
}

export function captureUploadMorph(
  ownerships: readonly RuntimeFeatureDirectiveOwnership[],
  activeFields: readonly string[],
): UploadMorphContinuity {
  if (activeFields.length > MAX_UPLOAD_FILES_PER_DOCUMENT) {
    throw new Error("upload_morph_field_limit");
  }
  const fields = new Set<string>();
  for (const field of activeFields) {
    validateUploadField(field);
    fields.add(field);
  }
  return Object.freeze({
    fields: Object.freeze([...fields].map((field) => describeField(ownerships, field))),
  });
}

export function reconcileUploadMorph(
  continuity: UploadMorphContinuity,
  ownerships: readonly RuntimeFeatureDirectiveOwnership[],
): readonly string[] {
  const compatible: string[] = [];
  for (const previous of continuity.fields) {
    if (!previous.valid) continue;
    const current = describeField(ownerships, previous.field);
    if (!current.valid || current.elements.length !== previous.elements.length) continue;
    const matches = previous.elements.every((item, index) => {
      const candidate = current.elements[index];
      return (
        candidate?.element === item.element &&
        candidate.key === item.key &&
        candidate.token === item.token
      );
    });
    if (matches) compatible.push(previous.field);
  }
  return Object.freeze(compatible);
}
