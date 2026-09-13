# Explicit registration and reserved namespaces for the component library

Level: Consequential
Decided by: Shawn
Rests on: UI-014, UI-015, UI-016, UI-018, UI-019, LIVE-005, LIVE-006
Would be wrong if: an application cannot mix the default library with its own or a third party's components without a name, template, style, or tag collision, or a library component becomes reachable without the application naming it in its registry builder

## Decision

The library rides Live's existing explicit registry: an application registers each library behavioral component through LiveRegistry::builder().register::<T>(), the registry is immutable after build, and duplicate names or views fail with a typed error at boot. No default set registers itself and no convenience registers everything. The namespaces a default library could share with an application's or a third party's components are reserved: component names carry the suprnova. prefix, views live under the suprnova-ui/ template root, the stylesheet sits in the suprnova-ui cascade layer with --sn- tokens so unlayered application CSS wins by cascade order, custom-element tags carry the sn- prefix and are defined only for registered components, and library assets load only when a document opts in through LiveBootstrapOptions, the way the Stimulus role does. Decided by Shawn on 2026-09-13 at 09:57 after the explicit-list proposal was checked against how Live registers components today.

## Realized by

- d56fb7ce  docs(cairn): split the library specification by component family and record the registration ruling
