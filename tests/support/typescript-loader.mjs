export async function resolve(specifier, context, nextResolve) {
  if (
    context.parentURL?.endsWith(".ts") === true &&
    specifier.startsWith(".") &&
    specifier.endsWith(".js")
  ) {
    return {
      shortCircuit: true,
      url: new URL(`${specifier.slice(0, -3)}.ts`, context.parentURL).href,
    };
  }
  return nextResolve(specifier, context);
}
