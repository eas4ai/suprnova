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

const TARGET_KINDS = ["island", "keyed_scope", "element"];
const LITERAL_KINDS = ["boolean", "integer", "string", "token", "mapping"];
const ARGUMENT_FORMS = [
  "none",
  "identifier",
  "field",
  "action",
  "target",
  "mapping",
];
const FALLBACKS = ["inert", "native", "retain_dom"];
const VALUE_KINDS = [
  "empty",
  "identifier",
  "literal",
  "field",
  "action",
  "target",
  "mapping",
];
const DIRECTIVE_PHASES = [
  "local",
  "schedule",
  "feedback",
  "morph",
  "navigation",
];
const CAPABILITIES = ["uploads@1", "async@1"];
const MODIFIER_GROUPS = [
  "event",
  "model",
  "feedback",
  "morph",
  "transition",
  "navigation",
];
const MAX_MODIFIER_SEGMENTS = 3;
const FRESHNESS_STREAM_MODES = ["absent", "default", "hybrid", "push-only"];
const FRESHNESS_RESULTS = [
  "none",
  "poll_only",
  "hybrid_descriptor",
  "hybrid_poll_override",
  "push_only",
  "directive_conflict",
];
const VALUE_GRAMMAR = {
  token: {
    maximum_bytes: 64,
    initial: "ascii_lowercase",
    continuation: ["ascii_lowercase", "ascii_digit", "_", ".", ":", "-"],
  },
  integer: {
    canonical: true,
    maximum_absolute: "9007199254740991",
  },
};

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

function exactFields(value, fields, label, optionalFields = []) {
  const allowed = new Set([...fields, ...optionalFields]);
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

function exactOrderedStrings(value, expected, label) {
  const entries = boundedUniqueStrings(
    value,
    label,
    expected.length,
    /^[a-z][a-z0-9_]{0,31}$/,
  );
  if (JSON.stringify(entries) !== JSON.stringify(expected)) {
    throw new TypeError(`invalid_${label}`);
  }
  return entries;
}

function modifierStrings(value, label) {
  const modifiers = boundedUniqueStrings(
    value,
    label,
    64,
    /^[a-z0-9][a-z0-9_.-]{0,63}$/,
  );
  if (
    modifiers.some(
      (modifier) => modifier.split(".").length > MAX_MODIFIER_SEGMENTS,
    )
  ) {
    throw new TypeError(`too_deep_${label}`);
  }
  return modifiers;
}

function validateValueGrammar(value) {
  try {
    const grammar = object(value, "value_grammar");
    exactFields(grammar, ["token", "integer"], "value_grammar");
    const token = object(grammar["token"], "value_grammar_token");
    exactFields(
      token,
      ["maximum_bytes", "initial", "continuation"],
      "value_grammar_token",
    );
    const integer = object(grammar["integer"], "value_grammar_integer");
    exactFields(
      integer,
      ["canonical", "maximum_absolute"],
      "value_grammar_integer",
    );
    if (
      token["maximum_bytes"] !== VALUE_GRAMMAR.token.maximum_bytes ||
      token["initial"] !== VALUE_GRAMMAR.token.initial ||
      JSON.stringify(token["continuation"]) !==
        JSON.stringify(VALUE_GRAMMAR.token.continuation) ||
      integer["canonical"] !== VALUE_GRAMMAR.integer.canonical ||
      integer["maximum_absolute"] !== VALUE_GRAMMAR.integer.maximum_absolute
    ) {
      throw new TypeError("invalid_value_grammar");
    }
    return grammar;
  } catch {
    throw new TypeError("invalid_value_grammar");
  }
}

function validateSyntax(grammar, schemaVersion) {
  const syntax = object(grammar["syntax"], "syntax");
  const fields = [
    "prefix",
    "target_kinds",
    "literal_kinds",
    "argument_forms",
    "fallbacks",
  ];
  if (schemaVersion === 2) fields.push("value_kinds", "value_grammar");
  exactFields(syntax, fields, "syntax");
  if (syntax["prefix"] !== "live:") {
    throw new TypeError("invalid_directive_prefix");
  }
  exactOrderedStrings(syntax["target_kinds"], TARGET_KINDS, "target_kinds");
  exactOrderedStrings(syntax["literal_kinds"], LITERAL_KINDS, "literal_kinds");
  exactOrderedStrings(
    syntax["argument_forms"],
    ARGUMENT_FORMS,
    "argument_forms",
  );
  if (schemaVersion === 2) {
    exactOrderedStrings(syntax["value_kinds"], VALUE_KINDS, "value_kinds");
  }
  exactOrderedStrings(syntax["fallbacks"], FALLBACKS, "fallbacks");
  if (schemaVersion === 2) validateValueGrammar(syntax["value_grammar"]);
  return syntax;
}

function validateModifierGroups(grammar) {
  for (const group of MODIFIER_GROUPS) {
    modifierStrings(grammar[`${group}_modifiers`], `${group}_modifiers`);
  }
}

export function loadFreshnessCombinations(grammar) {
  const entries = grammar["freshness_combinations"];
  const expectedCount = FRESHNESS_STREAM_MODES.length * 2;
  if (!Array.isArray(entries) || entries.length !== expectedCount) {
    throw new TypeError("invalid_freshness_combinations");
  }
  const combinations = entries.map((entry, index) => {
    const combination = object(entry, `freshness_combination_${String(index)}`);
    exactFields(
      combination,
      ["poll", "stream", "result"],
      `freshness_combination_${String(index)}`,
    );
    const poll = combination["poll"];
    const stream = string(combination["stream"], "freshness_stream");
    const result = string(combination["result"], "freshness_result");
    if (
      typeof poll !== "boolean" ||
      !FRESHNESS_STREAM_MODES.includes(stream) ||
      !FRESHNESS_RESULTS.includes(result) ||
      !validFreshnessResult(poll, stream, result)
    ) {
      throw new TypeError("invalid_freshness_combinations");
    }
    return [poll, stream, result];
  });
  const keys = combinations.map(
    ([poll, stream]) => `${String(poll)}:${stream}`,
  );
  const expectedKeys = FRESHNESS_STREAM_MODES.flatMap((stream) => [
    `false:${stream}`,
    `true:${stream}`,
  ]);
  if (
    new Set(keys).size !== expectedCount ||
    expectedKeys.some((key) => !keys.includes(key))
  ) {
    throw new TypeError("invalid_freshness_combinations");
  }
  return combinations;
}

function validFreshnessResult(poll, stream, result) {
  if (result === "directive_conflict") return poll || stream !== "absent";
  if (result === "none") return !poll && stream === "absent";
  if (result === "poll_only") return poll && stream === "absent";
  if (result === "push_only") return !poll && stream === "push-only";
  if (result === "hybrid_descriptor") {
    return !poll && (stream === "default" || stream === "hybrid");
  }
  return (
    result === "hybrid_poll_override" &&
    poll &&
    (stream === "default" || stream === "hybrid")
  );
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
    return modifierStrings(value, `${directive}_modifiers`);
  }
  const group = string(value, `${directive}_modifier_group`);
  const groupField = `${group}_modifiers`;
  if (!Object.hasOwn(grammar, groupField)) {
    throw new TypeError(`unknown_${directive}_modifier_group_${group}`);
  }
  return modifierStrings(grammar[groupField], groupField);
}

function resolveModifierConflicts(value, directive, modifiers) {
  if (value === undefined) return [];
  if (!Array.isArray(value) || value.length > 16) {
    throw new TypeError(`invalid_${directive}_modifier_conflicts`);
  }
  const seen = new Set();
  return value.map((entry, index) => {
    const group = boundedUniqueStrings(
      entry,
      `${directive}_modifier_conflict_${String(index)}`,
      16,
      /^[a-z0-9][a-z0-9_.-]{0,63}$/,
    );
    if (group.length < 2) {
      throw new TypeError(`invalid_${directive}_modifier_conflict_group`);
    }
    for (const modifier of group) {
      if (!modifiers.includes(modifier)) {
        throw new TypeError(
          `unknown_${directive}_modifier_conflict_${modifier}`,
        );
      }
      if (seen.has(modifier)) {
        throw new TypeError(
          `duplicate_${directive}_modifier_conflict_${modifier}`,
        );
      }
      seen.add(modifier);
    }
    return group;
  });
}

export function loadContracts(grammar) {
  const allowedOwners = new Set(TARGET_KINDS);
  const allowedValues = new Set(VALUE_KINDS);
  const allowedPhases = new Set(DIRECTIVE_PHASES);
  const allowedFallbacks = new Set(FALLBACKS);
  const allowedCapabilities = new Set(CAPABILITIES);
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
    "freshness_combinations",
    "directives",
  ];
  exactFields(grammar, grammarFields, "grammar");
  if (grammar["schema_version"] !== 2 || grammar["contract_version"] !== 2) {
    throw new TypeError("unsupported_directive_contract");
  }
  validateSyntax(grammar, 2);
  validateModifierGroups(grammar);
  loadFreshnessCombinations(grammar);
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
      ["modifier_conflicts"],
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
    const modifierConflicts = resolveModifierConflicts(
      descriptor["modifier_conflicts"],
      name,
      modifiers,
    );
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
      modifierConflicts,
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
  if (
    previousGrammar["schema_version"] !== 1 ||
    previousGrammar["contract_version"] !== 1
  ) {
    throw new TypeError("unsupported_v3_directive_contract");
  }
  validateSyntax(previousGrammar, 1);
  validateModifierGroups(previousGrammar);
  const validatedContracts = loadContracts(grammar);
  if (JSON.stringify(contracts) !== JSON.stringify(validatedContracts)) {
    throw new TypeError("inconsistent_v4_contracts");
  }
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
      current.modifierConflicts.length !== 0 ||
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
  const valueGrammar = object(syntax["value_grammar"], "value_grammar");
  const tokenGrammar = object(valueGrammar["token"], "value_grammar_token");
  const integerGrammar = object(
    valueGrammar["integer"],
    "value_grammar_integer",
  );
  const tokenMaximumBytes = tokenGrammar["maximum_bytes"];
  const integerMaximumAbsolute = string(
    integerGrammar["maximum_absolute"],
    "value_grammar_integer_maximum",
  );
  const targetKinds = strings(syntax["target_kinds"], "target_kinds");
  const literalKinds = strings(syntax["literal_kinds"], "literal_kinds");
  const argumentForms = strings(syntax["argument_forms"], "argument_forms");
  const valueKinds = strings(syntax["value_kinds"], "value_kinds");
  const fallbacks = strings(syntax["fallbacks"], "fallbacks");
  const reserved = strings(grammar["reserved"], "reserved");
  const freshnessCombinations = loadFreshnessCombinations(grammar);
  const valueVariants = valueKinds
    .map((value) => `    ${variant(value)},`)
    .join("\n");
  const descriptors = contracts
    .map(
      (contract) =>
        `    DirectiveContract { name: ${JSON.stringify(contract.name)}, owner: DirectiveOwner::${variant(contract.owner)}, value: DirectiveValue::${variant(contract.value)}, modifiers: &[${quotedList(contract.modifiers)}], modifier_conflicts: &[${contract.modifierConflicts.map((group) => `&[${quotedList(group)}]`).join(", ")}], roles: &[${quotedList(contract.roles)}], conflicts: &[${quotedList(contract.conflicts)}], phase: DirectivePhase::${variant(contract.phase)}, fallback: DirectiveFallback::${variant(contract.fallback)}, capability: ${rustOption(contract.capability)} },`,
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
${valueVariants}
}

const DIRECTIVE_VALUE_TOKEN_MAXIMUM_BYTES: usize = ${String(tokenMaximumBytes)};
const DIRECTIVE_VALUE_INTEGER_MAXIMUM_ABSOLUTE: &str = ${JSON.stringify(integerMaximumAbsolute)};

fn valid_directive_value_token(value: &str) -> bool {
    let mut bytes = value.bytes();
    value.len() <= DIRECTIVE_VALUE_TOKEN_MAXIMUM_BYTES
        && bytes.next().is_some_and(|first| first.is_ascii_lowercase())
        && bytes.all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'_' | b'.' | b':' | b'-')
        })
}

fn valid_directive_value_integer(value: &str) -> bool {
    let (negative, digits) = value
        .strip_prefix('-')
        .map_or((false, value), |digits| (true, digits));
    if digits == "0" {
        return !negative;
    }
    if digits.is_empty()
        || digits.starts_with('0')
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
    {
        return false;
    }
    digits.len() < DIRECTIVE_VALUE_INTEGER_MAXIMUM_ABSOLUTE.len()
        || (digits.len() == DIRECTIVE_VALUE_INTEGER_MAXIMUM_ABSOLUTE.len()
            && digits <= DIRECTIVE_VALUE_INTEGER_MAXIMUM_ABSOLUTE)
}

#[must_use]
pub fn valid_directive_scalar_value(value_kind: DirectiveValue, value: &str) -> Option<bool> {
    match value_kind {
        DirectiveValue::Identifier | DirectiveValue::Field | DirectiveValue::Action => {
            Some(valid_directive_value_token(value))
        }
        DirectiveValue::Literal => Some(
            valid_directive_value_token(value)
                || matches!(value, "true" | "false" | "null")
                || valid_directive_value_integer(value),
        ),
        DirectiveValue::Empty | DirectiveValue::Target | DirectiveValue::Mapping => None,
    }
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
    pub modifier_conflicts: &'static [&'static [&'static str]],
    pub roles: &'static [&'static str],
    pub conflicts: &'static [&'static str],
    pub phase: DirectivePhase,
    pub fallback: DirectiveFallback,
    pub capability: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FreshnessCombination {
    pub poll: bool,
    pub stream: &'static str,
    pub result: &'static str,
}

#[rustfmt::skip]
pub const FRESHNESS_COMBINATIONS: &[FreshnessCombination] = &[
${freshnessCombinations.map(([poll, stream, result]) => `    FreshnessCombination { poll: ${String(poll)}, stream: ${JSON.stringify(stream)}, result: ${JSON.stringify(result)} },`).join("\n")}
];

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

export function renderTypeScript(grammar, manifest, contracts) {
  const syntax = object(grammar["syntax"], "syntax");
  const valueGrammar = object(syntax["value_grammar"], "value_grammar");
  const tokenGrammar = object(valueGrammar["token"], "value_grammar_token");
  const integerGrammar = object(
    valueGrammar["integer"],
    "value_grammar_integer",
  );
  const tokenMaximumBytes = tokenGrammar["maximum_bytes"];
  const integerMaximumAbsolute = string(
    integerGrammar["maximum_absolute"],
    "value_grammar_integer_maximum",
  );
  const targetKinds = strings(syntax["target_kinds"], "target_kinds");
  const literalKinds = strings(syntax["literal_kinds"], "literal_kinds");
  const argumentForms = strings(syntax["argument_forms"], "argument_forms");
  const valueKinds = strings(syntax["value_kinds"], "value_kinds");
  const fallbacks = strings(syntax["fallbacks"], "fallbacks");
  const reserved = strings(grammar["reserved"], "reserved");
  const freshnessCombinations = loadFreshnessCombinations(grammar);
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
        modifierConflicts: [${contract.modifierConflicts.map((group) => `[${quotedList(group)}]`).join(", ")}],
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
      const valueCode = valueKinds.indexOf(contract.value);
      const fallbackCode = fallbacks.indexOf(contract.fallback);
      return feature
        ? `  [${JSON.stringify(contract.name)}, ${String(valueCode)}, ${sharedArray(contract.modifiers)}, ${sharedArray(contract.roles)}, ${sharedArray(contract.conflicts)}, [${contract.modifierConflicts.map((group) => `[${quotedList(group)}]`).join(", ")}], ${String(fallbackCode)}, ${JSON.stringify(contract.capability)}],`
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

export type DirectiveOwner = ${TARGET_KINDS.map((value) => JSON.stringify(value)).join(" | ")};
export type DirectiveValue = ${valueKinds.map((value) => JSON.stringify(value)).join(" | ")};
export type DirectivePhase = ${DIRECTIVE_PHASES.map((value) => JSON.stringify(value)).join(" | ")};
export type DirectiveFallback = ${fallbacks.map((value) => JSON.stringify(value)).join(" | ")};
export type DirectiveCapability = ${CAPABILITIES.map((value) => JSON.stringify(value)).join(" | ")};
export type FreshnessStreamMode = "absent" | "default" | "hybrid" | "push-only";
export type FreshnessCombinationResult =
  | "none"
  | "poll_only"
  | "hybrid_descriptor"
  | "hybrid_poll_override"
  | "push_only"
  | "directive_conflict";

export interface DirectiveContract {
  readonly name: string;
  readonly owner: DirectiveOwner;
  readonly value: DirectiveValue;
  readonly modifiers: readonly string[];
  readonly roles: readonly string[];
  readonly conflicts: readonly string[];
  readonly modifierConflicts: readonly (readonly string[])[];
  readonly phase: DirectivePhase;
  readonly fallback: DirectiveFallback;
  readonly capability: DirectiveCapability | null;
}

export type RuntimeDirectiveContract = readonly [
  name: string,
  value: ${valueKinds.map((_, index) => String(index)).join(" | ")},
  modifiers: readonly string[],
  conflicts: readonly string[],
  fallback: ${fallbacks.map((_, index) => String(index)).join(" | ")},
];

export type FeatureDirectiveContract = readonly [
  name: string,
  value: ${valueKinds.map((_, index) => String(index)).join(" | ")},
  modifiers: readonly string[],
  roles: readonly string[],
  conflicts: readonly string[],
  modifierConflicts: readonly (readonly string[])[],
  fallback: ${fallbacks.map((_, index) => String(index)).join(" | ")},
  capability: DirectiveCapability,
];

export const DIRECTIVE_FIXTURE_MANIFEST_SHA256 = ${JSON.stringify(manifest)};

const DIRECTIVE_VALUE_TOKEN_MAXIMUM_BYTES = ${String(tokenMaximumBytes)};
const DIRECTIVE_VALUE_INTEGER_MAXIMUM_ABSOLUTE = ${JSON.stringify(integerMaximumAbsolute)};
const DIRECTIVE_VALUE_TOKEN = /^[a-z][a-z0-9_.:-]*$/u;
const DIRECTIVE_VALUE_INTEGER = /^[0-9]+$/u;

function validDirectiveValueToken(value: string): boolean {
  return value.length <= DIRECTIVE_VALUE_TOKEN_MAXIMUM_BYTES && DIRECTIVE_VALUE_TOKEN.test(value);
}

function validDirectiveValueInteger(value: string): boolean {
  const negative = value.startsWith("-");
  const digits = negative ? value.slice(1) : value;
  if (digits === "0") return !negative;
  if (digits.length === 0 || digits.startsWith("0") || !DIRECTIVE_VALUE_INTEGER.test(digits)) {
    return false;
  }
  return (
    digits.length < DIRECTIVE_VALUE_INTEGER_MAXIMUM_ABSOLUTE.length ||
    (digits.length === DIRECTIVE_VALUE_INTEGER_MAXIMUM_ABSOLUTE.length &&
      digits <= DIRECTIVE_VALUE_INTEGER_MAXIMUM_ABSOLUTE)
  );
}

export function validDirectiveScalarValue(
  valueKind: RuntimeDirectiveContract[1],
  value: string,
): boolean | undefined {
  switch (valueKind) {
    case 1:
    case 3:
    case 4:
      return validDirectiveValueToken(value);
    case 2:
      return (
        validDirectiveValueToken(value) ||
        value === "true" ||
        value === "false" ||
        value === "null" ||
        validDirectiveValueInteger(value)
      );
    case 0:
    case 5:
    case 6:
      return undefined;
  }
}

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

// One generated authority for the legal poll/stream freshness combinations.
// prettier-ignore
const FRESHNESS_COMBINATIONS = [
${freshnessCombinations.map(([poll, stream, result]) => `  [${String(poll)}, ${JSON.stringify(stream)}, ${JSON.stringify(result)}],`).join("\n")}
] as const satisfies readonly (readonly [boolean, FreshnessStreamMode, FreshnessCombinationResult])[];

export function freshnessCombination(
  poll: boolean,
  stream: FreshnessStreamMode,
): FreshnessCombinationResult | undefined {
  return FRESHNESS_COMBINATIONS.find(
    (combination) => combination[0] === poll && combination[1] === stream,
  )?.[2];
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
