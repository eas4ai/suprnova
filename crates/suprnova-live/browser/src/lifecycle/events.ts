export type DocumentLifecycleEvent =
  | Readonly<{ kind: "freeze" }>
  | Readonly<{ kind: "pagehide"; persisted: boolean }>
  | Readonly<{ kind: "pageshow"; persisted: boolean }>
  | Readonly<{ kind: "resume" }>;

export interface DocumentLifecycleEventSources {
  readonly document: EventTarget;
  readonly supportsFreezeResume: boolean;
  readonly window: EventTarget;
}

function persisted(event: Event): boolean {
  return Reflect.get(event, "persisted") === true;
}

export function normalizeDocumentLifecycleEvent(event: Event): DocumentLifecycleEvent | null {
  switch (event.type) {
    case "freeze":
      return Object.freeze({ kind: "freeze" });
    case "pagehide":
      return Object.freeze({ kind: "pagehide", persisted: persisted(event) });
    case "pageshow":
      return Object.freeze({ kind: "pageshow", persisted: persisted(event) });
    case "resume":
      return Object.freeze({ kind: "resume" });
    default:
      return null;
  }
}

export function supportsDocumentFreezeResume(document: Document): boolean {
  return Reflect.has(document, "onfreeze") || Reflect.has(document, "onresume");
}
