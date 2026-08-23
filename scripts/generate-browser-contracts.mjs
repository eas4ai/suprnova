import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const fixtureDirectory = resolve(repositoryRoot, "fixtures/v4");
const grammarPath = resolve(fixtureDirectory, "directive-grammar.json");
const manifestPath = resolve(fixtureDirectory, "manifest.sha256");
const previousGrammarPath = resolve(
  repositoryRoot,
  "fixtures/v3/directive-grammar.json",
);
const rustOutput = resolve(
  repositoryRoot,
  "src/checker/generated_directive_contract.rs",
);
const typescriptOutput = resolve(
  repositoryRoot,
  "browser/src/generated/directive-contract.ts",
);
const checkOnly = process.argv.slice(2).includes("--check");

function object(value, label) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new TypeError(`invalid_${label}`);
  }
  return value;
}

function string(value, label) {
  if (typeof value !== "string") throw new TypeError(`invalid_${label}`);
  return value;
}

function strings(value, label) {
  if (
    !Array.isArray(value) ||
    !value.every((entry) => typeof entry === "string")
  ) {
    throw new TypeError(`invalid_${label}`);
  }
  return value;
}

function exactFields(value, fields, label) {
  const allowed = new Set(fields);
  for (const field of fields) {
    if (!Object.hasOwn(value, field))
      throw new TypeError(`missing_${label}_field_${field}`);
  }
  for (const field of Object.keys(value)) {
    if (!allowed.has(field))
      throw new TypeError(`unknown_${label}_field_${field}`);
  }
}

function boundedUniqueStrings(value, label, maximumItems, pattern) {
  const entries = strings(value, label);
  if (entries.length > maximumItems) throw new TypeError(`too_many_${label}`);
  if (new Set(entries).size !== entries.length)
    throw new TypeError(`duplicate_${label}`);
  if (entries.some((entry) => !pattern.test(entry)))
    throw new TypeError(`invalid_${label}`);
  return entries;
}

function variant(value) {
  return value
    .split("_")
    .map((part) => `${part.slice(0, 1).toUpperCase()}${part.slice(1)}`)
    .join("");
}

function quotedList(values) {
  return values.map((value) => JSON.stringify(value)).join(", ");
}

function rustOption(value) {
  return value === null ? "None" : `Some(${JSON.stringify(value)})`;
}

function resolveModifiers(grammar, value, directive) {
  if (Array.isArray(value)) {
    return boundedUniqueStrings(
      value,
      `${directive}_modifiers`,
      64,
      /^[a-z0-9][a-z0-9_.-]{0,63}$/,
    );
  }
  const group = string(value, `${directive}_modifier_group`);
  const groupField = `${group}_modifiers`;
  if (!Object.hasOwn(grammar, groupField)) {
    throw new TypeError(`unknown_${directive}_modifier_group_${group}`);
  }
  return boundedUniqueStrings(
    grammar[groupField],
    groupField,
    64,
    /^[a-z0-9][a-z0-9_.-]{0,63}$/,
  );
}

export function loadContracts(grammar) {
  const allowedOwners = new Set(["island", "keyed_scope", "element"]);
  const allowedValues = new Set([
    "empty",
    "identifier",
    "literal",
    "field",
    "action",
    "target",
    "mapping",
  ]);
  const allowedPhases = new Set([
    "local",
    "schedule",
    "feedback",
    "morph",
    "navigation",
  ]);
  const allowedFallbacks = new Set(["inert", "native", "retain_dom"]);
  const allowedCapabilities = new Set(["uploads@1", "async@1"]);
  const grammarFields = [
    "schema_version",
    "contract_version",
    "syntax",
    "event_modifiers",
    "model_modifiers",
    "feedback_modifiers",
    "morph_modifiers",
    "transition_modifiers",
    "navigation_modifiers",
    "reserved",
    "directives",
  ];
  exactFields(grammar, grammarFields, "grammar");
  if (grammar["schema_version"] !== 2 || grammar["contract_version"] !== 2) {
    throw new TypeError("unsupported_directive_contract");
  }
  const syntax = object(grammar["syntax"], "syntax");
  exactFields(
    syntax,
    ["prefix", "target_kinds", "literal_kinds", "argument_forms", "fallbacks"],
    "syntax",
  );
  if (syntax["prefix"] !== "live:")
    throw new TypeError("invalid_directive_prefix");
  boundedUniqueStrings(
    syntax["target_kinds"],
    "target_kinds",
    8,
    /^[a-z][a-z0-9_]{0,31}$/,
  );
  boundedUniqueStrings(
    syntax["literal_kinds"],
    "literal_kinds",
    16,
    /^[a-z][a-z0-9_]{0,31}$/,
  );
  boundedUniqueStrings(
    syntax["argument_forms"],
    "argument_forms",
    16,
    /^[a-z][a-z0-9_]{0,31}$/,
  );
  boundedUniqueStrings(
    syntax["fallbacks"],
    "fallbacks",
    8,
    /^[a-z][a-z0-9_]{0,31}$/,
  );
  for (const group of [
    "event",
    "model",
    "feedback",
    "morph",
    "transition",
    "navigation",
  ]) {
    boundedUniqueStrings(
      grammar[`${group}_modifiers`],
      `${group}_modifiers`,
      64,
      /^[a-z0-9][a-z0-9_.-]{0,63}$/,
    );
  }
  const reserved = boundedUniqueStrings(
    grammar["reserved"],
    "reserved_directives",
    64,
    /^[a-z][a-z0-9_-]{0,31}$/,
  );
  const directives = grammar["directives"];
  if (
    !Array.isArray(directives) ||
    directives.length === 0 ||
    directives.length > 64
  ) {
    throw new TypeError("invalid_directives");
  }
  const names = new Set();
  const contracts = directives.map((entry, index) => {
    const descriptor = object(entry, `directive_${String(index)}`);
    exactFields(
      descriptor,
      [
        "name",
        "owner",
        "value",
        "modifiers",
        "roles",
        "conflicts",
        "phase",
        "fallback",
        "capability",
      ],
      `directive_${String(index)}`,
    );
    const name = string(descriptor["name"], "directive_name");
    const owner = string(descriptor["owner"], `${name}_owner`);
    const value = string(descriptor["value"], `${name}_value`);
    const phase = string(descriptor["phase"], `${name}_phase`);
    const fallback = string(descriptor["fallback"], `${name}_fallback`);
    if (!/^[a-z][a-z0-9_-]{0,31}$/.test(name) || names.has(name)) {
      throw new TypeError("invalid_directive_name");
    }
    names.add(name);
    if (
      !allowedOwners.has(owner) ||
      !allowedValues.has(value) ||
      !allowedPhases.has(phase)
    ) {
      throw new TypeError(`invalid_directive_descriptor_${name}`);
    }
    if (!allowedFallbacks.has(fallback))
      throw new TypeError(`invalid_fallback_${name}`);
    const modifiers = resolveModifiers(grammar, descriptor["modifiers"], name);
    const roles = boundedUniqueStrings(
      descriptor["roles"],
      `${name}_roles`,
      16,
      /^[a-z][a-z0-9_-]{0,31}$/,
    );
    const conflicts = boundedUniqueStrings(
      descriptor["conflicts"],
      `${name}_conflicts`,
      16,
      /^[a-z][a-z0-9_-]{0,31}$/,
    );
    const capability = descriptor["capability"];
    if (
      capability !== null &&
      (typeof capability !== "string" || !allowedCapabilities.has(capability))
    ) {
      throw new TypeError(`invalid_capability_${name}`);
    }
    if (capability === null && roles.length !== 0) {
      throw new TypeError(`roles_without_capability_${name}`);
    }
    for (const role of roles) {
      if (modifiers.includes(role))
        throw new TypeError(`ambiguous_${name}_suffix_${role}`);
    }
    return {
      name,
      owner,
      value,
      modifiers,
      roles,
      conflicts,
      phase,
      fallback,
      capability,
    };
  });
  for (const contract of contracts) {
    for (const conflict of contract.conflicts) {
      if (!names.has(conflict) || conflict === contract.name) {
        throw new TypeError(`unknown_${contract.name}_conflict_${conflict}`);
      }
    }
  }
  for (const name of reserved) {
    if (names.has(name))
      throw new TypeError(`reserved_registered_directive_${name}`);
  }
  return contracts;
}

export function validateV4Evolution(previousGrammar, grammar, contracts) {
  const previousDirectives = previousGrammar["directives"];
  if (!Array.isArray(previousDirectives))
    throw new TypeError("invalid_v3_directives");
  const promotedNames = ["upload", "progress", "poll", "stream"];
  if (contracts.length !== previousDirectives.length + promotedNames.length) {
    throw new TypeError("invalid_v4_directive_count");
  }
  const actualPromotions = contracts
    .slice(previousDirectives.length)
    .map(({ name }) => name);
  if (JSON.stringify(actualPromotions) !== JSON.stringify(promotedNames)) {
    throw new TypeError("invalid_v4_promotions");
  }
  for (const [index, previousEntry] of previousDirectives.entries()) {
    const previous = object(previousEntry, `v3_directive_${String(index)}`);
    const current = contracts[index];
    const carried = {
      name: string(previous["name"], "v3_directive_name"),
      owner: string(previous["owner"], "v3_directive_owner"),
      value: string(previous["value"], "v3_directive_value"),
      modifiers: resolveModifiers(
        previousGrammar,
        previous["modifiers"],
        `v3_${String(index)}`,
      ),
      conflicts: strings(
        previous["conflicts"],
        `v3_${String(index)}_conflicts`,
      ),
      phase: string(previous["phase"], "v3_directive_phase"),
      fallback: string(previous["fallback"], "v3_directive_fallback"),
    };
    const currentCarried = {
      name: current.name,
      owner: current.owner,
      value: current.value,
      modifiers: current.modifiers,
      conflicts: current.conflicts,
      phase: current.phase,
      fallback: current.fallback,
    };
    if (
      JSON.stringify(currentCarried) !== JSON.stringify(carried) ||
      current.roles.length !== 0 ||
      current.capability !== null
    ) {
      throw new TypeError(`changed_v3_directive_${carried.name}`);
    }
  }
  if (strings(grammar["reserved"], "reserved").length !== 0) {
    throw new TypeError("invalid_v4_reserved_directives");
  }
}

export function partitionRuntimeContracts(contracts) {
  const core = contracts.filter(({ capability }) => capability === null);
  const features = contracts.filter(({ capability }) => capability !== null);
  const partitioned = [...core, ...features];
  const partitionedNames = new Set(partitioned.map(({ name }) => name));
  if (
    partitioned.length !== contracts.length ||
    partitionedNames.size !== contracts.length ||
    core.some(({ capability }) => capability !== null) ||
    features.some(({ capability }) => capability === null)
  ) {
    throw new TypeError("invalid_runtime_contract_partition");
  }
  return {
    core,
    features,
    coreReservedNames: [...features]
      .sort((left, right) => {
        if (left.capability < right.capability) return -1;
        if (left.capability > right.capability) return 1;
        return 0;
      })
      .map(({ name }) => name),
  };
}

function renderRust(grammar, manifest, contracts) {
  const syntax = object(grammar["syntax"], "syntax");
  const targetKinds = strings(syntax["target_kinds"], "target_kinds");
  const literalKinds = strings(syntax["literal_kinds"], "literal_kinds");
  const argumentForms = strings(syntax["argument_forms"], "argument_forms");
  const fallbacks = strings(syntax["fallbacks"], "fallbacks");
  const reserved = strings(grammar["reserved"], "reserved");
  const descriptors = contracts
    .map(
      (contract) =>
        `    DirectiveContract { name: ${JSON.stringify(contract.name)}, owner: DirectiveOwner::${variant(contract.owner)}, value: DirectiveValue::${variant(contract.value)}, modifiers: &[${quotedList(contract.modifiers)}], roles: &[${quotedList(contract.roles)}], conflicts: &[${quotedList(contract.conflicts)}], phase: DirectivePhase::${variant(contract.phase)}, fallback: DirectiveFallback::${variant(contract.fallback)}, capability: ${rustOption(contract.capability)} },`,
    )
    .join("\n");
  return `// @generated by scripts/generate-browser-contracts.mjs from fixtures/v4/directive-grammar.json.
// Do not edit by hand; run the generator instead.

#![allow(missing_docs, reason = "generated from the reviewed directive fixture")]

/// Reviewed v4 fixture-manifest identity used to generate this contract.
pub const DIRECTIVE_FIXTURE_MANIFEST_SHA256: &str =
    ${JSON.stringify(manifest)};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectiveOwner {
    Island,
    KeyedScope,
    Element,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectiveValue {
    Empty,
    Identifier,
    Literal,
    Field,
    Action,
    Target,
    Mapping,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectivePhase {
    Local,
    Schedule,
    Feedback,
    Morph,
    Navigation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectiveFallback {
    Inert,
    Native,
    RetainDom,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirectiveContract {
    pub name: &'static str,
    pub owner: DirectiveOwner,
    pub value: DirectiveValue,
    pub modifiers: &'static [&'static str],
    pub roles: &'static [&'static str],
    pub conflicts: &'static [&'static str],
    pub phase: DirectivePhase,
    pub fallback: DirectiveFallback,
    pub capability: Option<&'static str>,
}

#[rustfmt::skip]
pub const DIRECTIVE_CONTRACTS: &[DirectiveContract] = &[
${descriptors}
];

#[rustfmt::skip]
pub const RESERVED_DIRECTIVES: &[&str] = &[${quotedList(reserved)}];
#[rustfmt::skip]
pub const DIRECTIVE_TARGET_KINDS: &[&str] = &[${quotedList(targetKinds)}];
#[rustfmt::skip]
pub const DIRECTIVE_LITERAL_KINDS: &[&str] = &[${quotedList(literalKinds)}];
#[rustfmt::skip]
pub const DIRECTIVE_ARGUMENT_FORMS: &[&str] = &[${quotedList(argumentForms)}];
#[rustfmt::skip]
pub const DIRECTIVE_FALLBACKS: &[&str] = &[${quotedList(fallbacks)}];

#[must_use]
pub fn directive_contract(name: &str) -> Option<&'static DirectiveContract> {
    DIRECTIVE_CONTRACTS
        .iter()
        .find(|contract| contract.name == name)
}

#[must_use]
pub fn is_reserved_directive(name: &str) -> bool {
    RESERVED_DIRECTIVES.contains(&name)
}
`;
}

function renderTypeScript(grammar, manifest, contracts) {
  const syntax = object(grammar["syntax"], "syntax");
  const targetKinds = strings(syntax["target_kinds"], "target_kinds");
  const literalKinds = strings(syntax["literal_kinds"], "literal_kinds");
  const argumentForms = strings(syntax["argument_forms"], "argument_forms");
  const fallbacks = strings(syntax["fallbacks"], "fallbacks");
  const reserved = strings(grammar["reserved"], "reserved");
  const { core, features, coreReservedNames } =
    partitionRuntimeContracts(contracts);
  const coreReserved = [...reserved, ...coreReservedNames];
  const descriptors = contracts
    .map(
      (contract) => `  {
    name: ${JSON.stringify(contract.name)},
    owner: ${JSON.stringify(contract.owner)},
    value: ${JSON.stringify(contract.value)},
    modifiers: [${quotedList(contract.modifiers)}],
    roles: [${quotedList(contract.roles)}],
    conflicts: [${quotedList(contract.conflicts)}],
    phase: ${JSON.stringify(contract.phase)},
    fallback: ${JSON.stringify(contract.fallback)},
    capability: ${JSON.stringify(contract.capability)},
  },`,
    )
    .join("\n");
  const compactRuntime = (partition, prefix, feature) => {
    const sharedArrays = [];
    const sharedArrayIndexes = new Map();
    const sharedArray = (values) => {
      const key = JSON.stringify(values);
      let index = sharedArrayIndexes.get(key);
      if (index === undefined) {
        index = sharedArrays.length;
        sharedArrayIndexes.set(key, index);
        sharedArrays.push(values);
      }
      return `${prefix}${String(index)}`;
    };
    const runtimeDescriptors = partition.map((contract) => {
      const valueCode = [
        "empty",
        "identifier",
        "literal",
        "field",
        "action",
        "target",
        "mapping",
      ].indexOf(contract.value);
      const fallbackCode = ["inert", "native", "retain_dom"].indexOf(
        contract.fallback,
      );
      return feature
        ? `  [${JSON.stringify(contract.name)}, ${String(valueCode)}, ${sharedArray(contract.modifiers)}, ${sharedArray(contract.roles)}, ${sharedArray(contract.conflicts)}, ${String(fallbackCode)}, ${JSON.stringify(contract.capability)}],`
        : `  [${JSON.stringify(contract.name)}, ${String(valueCode)}, ${sharedArray(contract.modifiers)}, ${sharedArray(contract.conflicts)}, ${String(fallbackCode)}],`;
    });
    return {
      arrays: sharedArrays
        .map(
          (values, index) =>
            `const ${prefix}${String(index)} = [${quotedList(values)}] as const;`,
        )
        .join("\n"),
      descriptors: runtimeDescriptors.join("\n"),
    };
  };
  const coreRuntime = compactRuntime(core, "A", false);
  const featureRuntime = compactRuntime(features, "F", true);
  const eventTypes = contracts
    .filter(
      (contract) =>
        contract.phase === "schedule" &&
        contract.value === "action" &&
        contract.name !== "init" &&
        contract.capability === null,
    )
    .map((contract) => contract.name);
  return `// @generated by scripts/generate-browser-contracts.mjs from fixtures/v4/directive-grammar.json.
// Do not edit by hand; run the generator instead.

export type DirectiveOwner = "island" | "keyed_scope" | "element";
export type DirectiveValue = "empty" | "identifier" | "literal" | "field" | "action" | "target" | "mapping";
export type DirectivePhase = "local" | "schedule" | "feedback" | "morph" | "navigation";
export type DirectiveFallback = "inert" | "native" | "retain_dom";
export type DirectiveCapability = "uploads@1" | "async@1";

export interface DirectiveContract {
  readonly name: string;
  readonly owner: DirectiveOwner;
  readonly value: DirectiveValue;
  readonly modifiers: readonly string[];
  readonly roles: readonly string[];
  readonly conflicts: readonly string[];
  readonly phase: DirectivePhase;
  readonly fallback: DirectiveFallback;
  readonly capability: DirectiveCapability | null;
}

export type RuntimeDirectiveContract = readonly [
  name: string,
  value: 0 | 1 | 2 | 3 | 4 | 5 | 6,
  modifiers: readonly string[],
  conflicts: readonly string[],
  fallback: 0 | 1 | 2,
];

export type FeatureDirectiveContract = readonly [
  name: string,
  value: 0 | 1 | 2 | 3 | 4 | 5 | 6,
  modifiers: readonly string[],
  roles: readonly string[],
  conflicts: readonly string[],
  fallback: 0 | 1 | 2,
  capability: DirectiveCapability,
];

export const DIRECTIVE_FIXTURE_MANIFEST_SHA256 = ${JSON.stringify(manifest)};

export const DIRECTIVE_CONTRACTS = [
${descriptors}
] as const satisfies readonly DirectiveContract[];

// Production parsing uses the compact subset below. The complete reviewed descriptors above
// remain available to conformance tests without entering the production bundle.
// prettier-ignore
${coreRuntime.arrays}
// prettier-ignore
const RUNTIME_DIRECTIVE_CONTRACTS = [
${coreRuntime.descriptors}
] as const satisfies readonly RuntimeDirectiveContract[];

// prettier-ignore
export const DIRECTIVE_EVENT_TYPES = [${quotedList(eventTypes)}] as const;

// Capability directive names stay inert when their optional artifact is absent.
// prettier-ignore
export const CORE_RESERVED_DIRECTIVES = [${quotedList(coreReserved)}] as const;
// prettier-ignore
export const RESERVED_DIRECTIVES = [${quotedList(reserved)}] as const;
// prettier-ignore
export const DIRECTIVE_TARGET_KINDS = [${quotedList(targetKinds)}] as const;
// prettier-ignore
export const DIRECTIVE_LITERAL_KINDS = [${quotedList(literalKinds)}] as const;
// prettier-ignore
export const DIRECTIVE_ARGUMENT_FORMS = [${quotedList(argumentForms)}] as const;
// prettier-ignore
export const DIRECTIVE_FALLBACKS = [${quotedList(fallbacks)}] as const;

export function directiveContract(name: string): RuntimeDirectiveContract | undefined {
  return RUNTIME_DIRECTIVE_CONTRACTS.find((contract) => contract[0] === name);
}

export function isReservedDirective(name: string): boolean {
  return CORE_RESERVED_DIRECTIVES.some((candidate) => candidate === name);
}

// Optional artifacts consume this capability-only subset. Core production entries do not.
// prettier-ignore
${featureRuntime.arrays}
// prettier-ignore
const FEATURE_DIRECTIVE_CONTRACTS = [
${featureRuntime.descriptors}
] as const satisfies readonly FeatureDirectiveContract[];

export function featureDirectiveContract(name: string): FeatureDirectiveContract | undefined {
  return FEATURE_DIRECTIVE_CONTRACTS.find((contract) => contract[0] === name);
}
`;
}

async function expectedOutputs() {
  const grammar = object(
    JSON.parse(await readFile(grammarPath, "utf8")),
    "grammar",
  );
  const previousGrammar = object(
    JSON.parse(await readFile(previousGrammarPath, "utf8")),
    "previous_grammar",
  );
  const manifest = (await readFile(manifestPath, "utf8")).trim();
  if (!/^[0-9a-f]{64}$/.test(manifest))
    throw new TypeError("invalid_fixture_manifest");
  const contracts = loadContracts(grammar);
  validateV4Evolution(previousGrammar, grammar, contracts);
  const prettier = await import(
    new URL("../browser/node_modules/prettier/index.mjs", import.meta.url).href
  );
  const prettierConfig = (await prettier.resolveConfig(typescriptOutput)) ?? {};
  const typescript = await prettier.format(
    renderTypeScript(grammar, manifest, contracts),
    {
      ...prettierConfig,
      filepath: typescriptOutput,
    },
  );
  return new Map([
    [rustOutput, renderRust(grammar, manifest, contracts)],
    [typescriptOutput, typescript],
  ]);
}

async function main() {
  let drift = false;
  for (const [path, expected] of await expectedOutputs()) {
    if (checkOnly) {
      const actual = await readFile(path, "utf8").catch(() => "");
      if (actual !== expected) {
        process.stderr.write(`generated_contract_drift:${path}\n`);
        drift = true;
      }
    } else {
      await mkdir(dirname(path), { recursive: true });
      await writeFile(path, expected, "utf8");
    }
  }
  if (drift) process.exitCode = 1;
}

if (
  process.argv[1] !== undefined &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  await main();
}
