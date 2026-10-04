import type { RuntimeConfig } from "../runtime/types.js";
import { decodeSnapshotPublicView, type SnapshotForm } from "./snapshot-view.js";

export const ISLAND_ROOT_SELECTOR = "[data-suprnova-live-island]";
export const ISLAND_STATUS_ATTRIBUTE = "data-suprnova-live-status";
export const MAX_ISLANDS_PER_DOCUMENT = 10_000;

const MAX_METADATA_UNITS = 131_072;
const MAX_METADATA_ATTRIBUTES = 256;
const MAX_IDENTITY_UNITS = 128;
const MAX_UNSIGNED_64 = 18_446_744_073_709_551_615n;
const SAFE_TEXT_IDENTITY = /^[A-Za-z0-9._:/-]+$/u;
const SAFE_DOCUMENT_KEY = /^[A-Za-z0-9._:-]+$/u;
const SAFE_INSTANCE = /^[A-Za-z0-9_-]{22,43}$/u;
const ACTION_PARAMETERS_ATTRIBUTE = "data-suprnova-live-actions";
const ACTION_PARAMETER_ENTRY =
  /^([A-Za-z0-9._:/-]{1,128})\(([A-Za-z0-9._:/-]{1,128}(?:,[A-Za-z0-9._:/-]{1,128}){0,127})\)$/u;
const REQUIRED_ATTRIBUTES = [
  "data-suprnova-live-component",
  "data-suprnova-live-contract",
  "data-suprnova-live-document-key",
  "data-suprnova-live-lazy-complete",
  "data-suprnova-live-protocol-min",
  "data-suprnova-live-revision",
  "data-suprnova-live-root",
  "data-suprnova-live-slot",
  "data-suprnova-live-snapshot",
  "data-suprnova-live-snapshot-kind",
] as const;
const KNOWN_ATTRIBUTES = new Set([
  "data-suprnova-live-island",
  "data-suprnova-live-instance",
  ACTION_PARAMETERS_ATTRIBUTE,
  ISLAND_STATUS_ATTRIBUTE,
  ...REQUIRED_ATTRIBUTES,
]);

export interface IslandMetadata {
  readonly component: string;
  readonly slot: string;
  readonly documentKey: string;
  readonly protocolMinimum: 1 | 2;
  readonly runtimeContract: 1;
  readonly snapshot: Readonly<Record<string, unknown>>;
  readonly snapshotForm: SnapshotForm;
  readonly instanceId: string | null;
  readonly revision: bigint;
  readonly lazyComplete: boolean;
  /**
   * Each action that takes arguments, with its parameter names in declared
   * order: an action directive's positional literals are sent under these
   * names. Absent when no action of the component takes arguments.
   */
  readonly actionParameters?: ReadonlyMap<string, readonly string[]>;
}

export class IslandMetadataError extends Error {
  constructor(
    readonly kind: "invalid" | "incompatible",
    readonly detail: string,
  ) {
    super(`island_${kind}`);
    this.name = "IslandMetadataError";
  }
}

function fail(detail: string): never {
  throw new IslandMetadataError("invalid", detail);
}

function incompatible(detail: string): never {
  throw new IslandMetadataError("incompatible", detail);
}

function safeIdentity(value: string | null, pattern = SAFE_TEXT_IDENTITY): string {
  if (value === null || value.length > MAX_IDENTITY_UNITS || !pattern.test(value)) {
    return fail("identity");
  }
  return value;
}

function revision(value: string | null): bigint {
  if (value === null || value.length > 20 || !/^(0|[1-9][0-9]*)$/u.test(value)) {
    return fail("revision");
  }
  const parsed = BigInt(value);
  if (parsed > MAX_UNSIGNED_64) return fail("revision");
  return parsed;
}

function metadataUnits(element: Element): number {
  if (element.attributes.length > MAX_METADATA_ATTRIBUTES) return MAX_METADATA_UNITS + 1;
  let units = 0;
  for (const attribute of element.attributes) {
    if (!attribute.name.startsWith("data-suprnova-live-")) continue;
    units += attribute.name.length + attribute.value.length;
    if (units > MAX_METADATA_UNITS) return units;
  }
  return units;
}

function validateAttributeSet(element: Element): void {
  if (element.getAttribute("data-suprnova-live-island") !== "") fail("root_marker");
  if (metadataUnits(element) > MAX_METADATA_UNITS) fail("metadata_limit");
  for (const required of REQUIRED_ATTRIBUTES)
    if (!element.hasAttribute(required)) fail("attribute");
  for (const attribute of element.attributes) {
    if (
      attribute.name.startsWith("data-suprnova-live-") &&
      !KNOWN_ATTRIBUTES.has(attribute.name) &&
      !/^data-suprnova-live-flag-[a-z0-9_-]{1,32}$/u.test(attribute.name)
    ) {
      fail("attribute");
    }
  }
}

/**
 * Reads the root's `data-suprnova-live-actions` list: one
 * `action(parameter,parameter)` entry per action that takes arguments,
 * separated by single spaces. Anything else marks the island invalid.
 */
export function parseActionParameters(value: string): ReadonlyMap<string, readonly string[]> {
  const parameters = new Map<string, readonly string[]>();
  for (const entry of value.split(" ")) {
    const match = ACTION_PARAMETER_ENTRY.exec(entry);
    const action = match?.[1];
    const names = match?.[2]?.split(",");
    if (
      action === undefined ||
      names === undefined ||
      parameters.has(action) ||
      new Set(names).size !== names.length
    ) {
      return fail("action_parameters");
    }
    parameters.set(action, Object.freeze(names));
  }
  return parameters;
}

export function parseIslandMetadata(element: Element, config: RuntimeConfig): IslandMetadata {
  validateAttributeSet(element);
  const component = safeIdentity(element.getAttribute("data-suprnova-live-component"));
  const slot = safeIdentity(element.getAttribute("data-suprnova-live-slot"));
  if (element.getAttribute("data-suprnova-live-root") !== slot) fail("root_slot");
  const documentKey = safeIdentity(
    element.getAttribute("data-suprnova-live-document-key"),
    SAFE_DOCUMENT_KEY,
  );
  const contract = element.getAttribute("data-suprnova-live-contract");
  if (contract !== "1") incompatible("runtime_contract");
  const protocolText = element.getAttribute("data-suprnova-live-protocol-min");
  if (protocolText !== "1" && protocolText !== "2") incompatible("protocol");
  const protocolMinimum = Number(protocolText) as 1 | 2;
  if (protocolMinimum > config.protocol.maximum) incompatible("protocol");
  const snapshotForm = element.getAttribute("data-suprnova-live-snapshot-kind");
  if (snapshotForm !== "seed" && snapshotForm !== "instance") fail("snapshot_form");
  const acceptedRevision = revision(element.getAttribute("data-suprnova-live-revision"));
  const lazyText = element.getAttribute("data-suprnova-live-lazy-complete");
  if (lazyText !== "true" && lazyText !== "false") fail("lazy_complete");
  const encodedSnapshot = element.getAttribute("data-suprnova-live-snapshot");
  if (encodedSnapshot === null) fail("snapshot");
  let view;
  try {
    view = decodeSnapshotPublicView(encodedSnapshot);
  } catch {
    return fail("snapshot");
  }
  const rootInstance = element.getAttribute("data-suprnova-live-instance");
  const instanceId = snapshotForm === "instance" ? rootInstance : null;
  if (
    view.form !== snapshotForm ||
    view.component !== component ||
    view.slot !== slot ||
    view.revision !== acceptedRevision ||
    (snapshotForm === "seed" && (rootInstance !== null || acceptedRevision !== 0n)) ||
    (snapshotForm === "instance" &&
      (instanceId === null ||
        !SAFE_INSTANCE.test(instanceId) ||
        instanceId.length % 4 === 1 ||
        view.instanceId !== instanceId))
  ) {
    incompatible("snapshot_disagreement");
  }
  const actionText = element.getAttribute(ACTION_PARAMETERS_ATTRIBUTE);
  return Object.freeze({
    component,
    slot,
    documentKey,
    protocolMinimum,
    runtimeContract: 1,
    snapshot: view.envelope,
    snapshotForm,
    instanceId,
    revision: acceptedRevision,
    lazyComplete: lazyText === "true",
    ...(actionText === null ? {} : { actionParameters: parseActionParameters(actionText) }),
  });
}
