// script-builtin-class-static
class Resolvers extends Promise {}
Resolvers.withResolvers.call = () => ({}); // refused: script-builtin
