declare module "idiomorph" {
  export interface IdiomorphOptions {
    readonly morphStyle?: "outerHTML" | "innerHTML";
    readonly ignoreActive?: boolean;
    readonly ignoreActiveValue?: boolean;
    readonly restoreFocus?: boolean;
  }

  export const Idiomorph: {
    morph(
      oldNode: Element | Document,
      newContent: Element | Node | HTMLCollection | readonly Node[] | string | null,
      options?: IdiomorphOptions,
    ): readonly Node[] | undefined;
  };
}
