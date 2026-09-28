declare module "idiomorph" {
  interface IdiomorphCallbacks {
    readonly beforeAttributeUpdated?: (
      name: string,
      node: Element,
      mutation: "update" | "remove",
    ) => false | undefined;
    readonly beforeNodeAdded?: (node: Node) => false | undefined;
    readonly afterNodeAdded?: (node: Node) => void;
    readonly beforeNodeMorphed?: (current: Node, replacement: Node) => false | undefined;
    readonly afterNodeMorphed?: (current: Node, replacement: Node) => void;
    readonly beforeNodeRemoved?: (node: Node) => false | undefined;
    readonly afterNodeRemoved?: (node: Node) => void;
  }

  interface IdiomorphOptions {
    readonly morphStyle?: "outerHTML" | "innerHTML";
    readonly ignoreActive?: boolean;
    readonly ignoreActiveValue?: boolean;
    readonly restoreFocus?: boolean;
    readonly callbacks?: IdiomorphCallbacks;
  }

  export const Idiomorph: {
    morph(
      oldNode: Element | Document,
      newContent: Element | Node | HTMLCollection | readonly Node[] | string | null,
      options?: IdiomorphOptions,
    ): readonly Node[] | undefined;
  };
}
