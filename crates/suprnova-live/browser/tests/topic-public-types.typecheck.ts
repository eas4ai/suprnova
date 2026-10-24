import type { AsyncRuntimeIslandPort } from "@suprnova/live";
import type { TopicChangeSignal } from "@suprnova/live/async";

type Equal<A, B> =
  (<T>() => T extends A ? 1 : 2) extends <T>() => T extends B ? 1 : 2 ? true : false;
type Expect<T extends true> = T;
type DispatchedTopic = Parameters<NonNullable<AsyncRuntimeIslandPort["dispatchTopicChange"]>>[0];

export type SignalGeneration = Expect<Equal<TopicChangeSignal["generation"], number>>;
export type DispatchedGeneration = Expect<Equal<DispatchedTopic["generation"], number>>;
