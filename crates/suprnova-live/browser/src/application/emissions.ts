import type { ValidatedEmission } from "./types.js";

export interface EmissionSink {
  dispatch(emission: ValidatedEmission): void;
  effect(emission: ValidatedEmission): void | Promise<void>;
}

export function dispatchValidatedEvents(
  events: readonly ValidatedEmission[],
  sink: Pick<EmissionSink, "dispatch">,
): void {
  for (const event of events) sink.dispatch(event);
}

export async function runValidatedEffects(
  effects: readonly ValidatedEmission[],
  sink: Pick<EmissionSink, "effect">,
): Promise<void> {
  for (const effect of effects) await sink.effect(effect);
}
