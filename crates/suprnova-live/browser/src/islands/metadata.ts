import type { RuntimeConfig } from "../runtime/types.js";
import { decodeSnapshotPublicView, type SnapshotForm } from "./snapshot-view.js";

export const ISLAND_ROOT_SELECTOR = "[data-suprnova-live-island]";
export const ISLAND_STATUS_ATTRIBUTE = "data-suprnova-live-status";

const MAX_IDENTITY_UNITS = 128;
const MAX_UNSIGNED_64 = 18_446_744_073_709_551_615n;
const SAFE_TEXT_IDENTITY = /^[A-Za-z0-9._:/-]+$/u;
const SAFE_DOCUMENT_KEY = /^[A-Za-z0-9._:-]+$/u;
const SAFE_INSTANCE = /^[A-Za-z0-9_-]{22,43}$/u;
const ACTION_PARAMETERS_ATTRIBUTE = "data-suprnova-live-actions";
export const AGENT_TOOLS_ATTRIBUTE = "data-suprnova-live-tools";
const MAX_AGENT_TOOLS = 64;
const MAX_AGENT_FIELDS = 512;
const AGENT_TOOL_NAME = /^[A-Za-z0-9_.-]{1,128}$/u;
const AGENT_TOOL_INSTANCE = /^[A-Za-z0-9_-]{1,63}$/u;
const AGENT_FIELD_NAME = /^[A-Za-z0-9_]{1,128}$/u;
const AGENT_ANNOTATIONS = ["consequentialHint", "readOnlyHint", "untrustedContentHint"] as const;
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
  AGENT_TOOLS_ATTRIBUTE,
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
  /**
   * The component's actions the page registers as WebMCP tools, each named
   * for this island's document key, and the fields a tool result may show
   * (AGENT-012). Absent when the component offers the browser no tool.
   */
  readonly agentTools?: AgentToolContract;
}

/** The WebMCP hints a Live tool carries, all three always present. */
export interface AgentToolAnnotations {
  readonly readOnlyHint: boolean;
  readonly untrustedContentHint: boolean;
  readonly consequentialHint: boolean;
}

/** One Live action the page registers as a WebMCP tool. */
export interface AgentToolDefinition {
  readonly action: string;
  readonly name: string;
  readonly title?: string;
  readonly description: string;
  readonly inputSchema: Readonly<Record<string, unknown>>;
  readonly annotations: AgentToolAnnotations;
}

/** What the island root says the page registers for agents. */
export interface AgentToolContract {
  readonly fields: readonly string[];
  /**
   * The agent-visible fields with a 64-bit integer codec: a tool result
   * shows their lossless tags as decimal strings, and every other field as
   * the snapshot carries it.
   */
  readonly integers: readonly string[];
  readonly tools: readonly AgentToolDefinition[];
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

// The root's metadata, the signed snapshot above all, is what the server
// rendered under its own limits; the browser checks its grammar, not its size.
function validateAttributeSet(element: Element): void {
  if (element.getAttribute("data-suprnova-live-island") !== "") fail("root_marker");
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

function plainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * Reads the root's `data-suprnova-live-tools` contract. Every name must end
 * in a `.` and the island's document key, and the key must fit the tool
 * instance grammar, so a page never registers a name the server did not
 * build for this island. Anything else marks the island invalid.
 */
export function parseAgentTools(value: string, documentKey: string): AgentToolContract {
  let parsed: unknown;
  try {
    parsed = JSON.parse(value);
  } catch {
    return fail("agent_tools");
  }
  if (!plainObject(parsed) || parsed["v"] !== 1 || !AGENT_TOOL_INSTANCE.test(documentKey)) {
    return fail("agent_tools");
  }
  const keys = Object.keys(parsed).sort().join(",");
  const fields = parsed["fields"];
  const integers = parsed["integers"];
  const tools = parsed["tools"];
  if (
    keys !== "fields,integers,tools,v" ||
    !Array.isArray(fields) ||
    fields.length > MAX_AGENT_FIELDS ||
    !fields.every((field) => typeof field === "string" && AGENT_FIELD_NAME.test(field)) ||
    new Set(fields).size !== fields.length ||
    !Array.isArray(integers) ||
    !integers.every((field) => typeof field === "string" && fields.includes(field)) ||
    new Set(integers).size !== integers.length ||
    !Array.isArray(tools) ||
    tools.length === 0 ||
    tools.length > MAX_AGENT_TOOLS
  ) {
    return fail("agent_tools");
  }
  const names = new Set<string>();
  const definitions = tools.map((tool: unknown): AgentToolDefinition => {
    if (!plainObject(tool)) return fail("agent_tools");
    const { action, name, title, description, inputSchema, annotations } = tool;
    const allowed = new Set([
      "action",
      "annotations",
      "description",
      "inputSchema",
      "name",
      "title",
    ]);
    if (
      Object.keys(tool).some((key) => !allowed.has(key)) ||
      typeof action !== "string" ||
      action.length > MAX_IDENTITY_UNITS ||
      !SAFE_TEXT_IDENTITY.test(action) ||
      typeof name !== "string" ||
      !AGENT_TOOL_NAME.test(name) ||
      !name.endsWith(`.${documentKey}`) ||
      name.length === documentKey.length + 1 ||
      names.has(name) ||
      (title !== undefined && (typeof title !== "string" || title.trim().length === 0)) ||
      typeof description !== "string" ||
      description.trim().length === 0 ||
      !plainObject(inputSchema) ||
      !plainObject(annotations) ||
      Object.keys(annotations).sort().join(",") !== AGENT_ANNOTATIONS.join(",") ||
      !AGENT_ANNOTATIONS.every((hint) => typeof annotations[hint] === "boolean")
    ) {
      return fail("agent_tools");
    }
    names.add(name);
    return Object.freeze({
      action,
      name,
      ...(typeof title === "string" ? { title } : {}),
      description,
      inputSchema: Object.freeze(inputSchema),
      annotations: Object.freeze({
        readOnlyHint: annotations["readOnlyHint"] as boolean,
        untrustedContentHint: annotations["untrustedContentHint"] as boolean,
        consequentialHint: annotations["consequentialHint"] as boolean,
      }),
    });
  });
  return Object.freeze({
    fields: Object.freeze([...(fields as string[])]),
    integers: Object.freeze([...(integers as string[])]),
    tools: Object.freeze(definitions),
  });
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
    view = decodeSnapshotPublicView(encodedSnapshot, config.limits.maxJsonDepth);
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
  const toolsText = element.getAttribute(AGENT_TOOLS_ATTRIBUTE);
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
    ...(toolsText === null ? {} : { agentTools: parseAgentTools(toolsText, documentKey) }),
  });
}
