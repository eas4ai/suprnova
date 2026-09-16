/// The stable-key attribute a template writes: the one the checker validates
/// and the manual names (LIVE-024).
export const LIVE_KEY_ATTRIBUTE = "live:key";
/// The engine's own spelling of a stable key, on the roots the engine
/// renders and in documents written before `live:key` reached the runtime.
export const ENGINE_KEY_ATTRIBUTE = "data-suprnova-live-key";
/// The selector matching every element that declares a stable key.
export const KEYED_SELECTOR = "[live\\:key], [data-suprnova-live-key]";

/// Whether an element carries both spellings with different values: no
/// single identity, which identity planning refuses (`key_conflict`).
export function keyConflict(element: Element): boolean {
  const live = element.getAttribute(LIVE_KEY_ATTRIBUTE);
  const engine = element.getAttribute(ENGINE_KEY_ATTRIBUTE);
  return live !== null && engine !== null && live !== engine;
}

/// The stable key an element declares, from `live:key` first and the engine
/// spelling otherwise, or `null` for an unkeyed or a conflicting element.
export function stableKeyOf(element: Element): string | null {
  if (keyConflict(element)) return null;
  return element.getAttribute(LIVE_KEY_ATTRIBUTE) ?? element.getAttribute(ENGINE_KEY_ATTRIBUTE);
}
