export interface StimulusApplicationPort {
  start(): void;
  stop(): void;
  load(...definitions: readonly unknown[]): void;
  unload(...identifiers: readonly string[]): void;
}

export interface StimulusBootstrapOptions {
  readonly application: StimulusApplicationPort;
  readonly definitions?: readonly unknown[];
}

export interface StimulusContinuityRoot {
  readonly identity: string;
  readonly element: Element;
}

export interface StimulusContinuity {
  readonly scope: Element;
  readonly scopeIdentity: string | null;
  readonly roots: readonly StimulusContinuityRoot[];
}

export interface StimulusMorphBridge {
  beforeMorph(scope: Element): StimulusContinuity;
  afterMorph(continuity: StimulusContinuity, scope: Element): void;
  disposeScope(scope: Element): void;
  dispose(): void;
}
