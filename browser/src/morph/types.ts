export interface MorphLimits {
  readonly maxHtmlBytes: number;
  readonly maxNodes: number;
  readonly maxDepth: number;
  readonly maxAttributes: number;
  readonly maxAttributesPerElement: number;
  readonly maxKeys: number;
  readonly maxKeyBytes: number;
  readonly maxHookCalls: number;
  readonly deadlineMs: number;
}

export interface MorphAuthority {
  readonly component: string;
  readonly slot: string;
  readonly documentKey: string;
  readonly instanceId: string;
  readonly successorRevision: bigint;
  readonly encodedSnapshot: string;
}

export type MorphIdentityKind = "live_key" | "id" | "nested_island";

export interface MorphIdentityEntry {
  readonly kind: MorphIdentityKind;
  readonly value: string;
  readonly token: string;
  readonly current: Element | null;
  readonly replacement: Element | null;
  readonly currentPosition: string | null;
  readonly replacementPosition: string | null;
}

export interface IdentityPlan {
  readonly entries: readonly MorphIdentityEntry[];
  readonly nestedCurrentRoots: ReadonlySet<Element>;
  readonly nestedReplacementRoots: ReadonlySet<Element>;
  readonly moved: readonly string[];
  readonly inserted: readonly string[];
  readonly removed: readonly string[];
}

export interface MorphPlan {
  readonly currentRoot: HTMLElement;
  readonly replacementRoot: HTMLElement;
  readonly identity: IdentityPlan;
  readonly limits: MorphLimits;
}

export interface MorphResult {
  readonly root: HTMLElement;
  readonly moved: readonly string[];
  readonly inserted: readonly string[];
  readonly removed: readonly string[];
}

export interface MorphHooks {
  beforeMorph?(plan: MorphPlan): void;
  afterMorph?(result: MorphResult): void;
  beforeNodeAdded?(node: Node): void;
  afterNodeAdded?(node: Node): void;
  beforeNodeMorphed?(current: Node, replacement: Node): void;
  afterNodeMorphed?(current: Node, replacement: Node): void;
  beforeNodeRemoved?(node: Node): void;
  afterNodeRemoved?(node: Node): void;
}

export interface MorphAdapter {
  apply(plan: MorphPlan, hooks: MorphHooks): MorphResult;
}

export type MorphClock = () => number;
