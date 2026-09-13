# Behavior lives in the crate; views, macros, CSS, and JavaScript are vendored by live:add from a manifest

Level: Consequential
Decided by: Shawn
Rests on: UI-016, UI-017
Would be wrong if: a behavioral component's Rust and its vendored view drift across a framework upgrade without live:check catching it, or live:add overwrites an application's edit

## Decision

Option (c) of three, chosen 2026-09-13 10:23, refined at 10:36 with the component-directory unit. Behavioral components' Rust lives in the in-tree library crate and moves with the framework tag under the gate, budgets, and qualification harness. Views, presentational macros, and per-component CSS and JavaScript are vendored into the application under the reserved suprnova-ui/ template root by live:add, which reads one JSON manifest per component (the Livewire 4 multi-file component and the shadcn registry item were the models the developer cited) and never overwrites an edited file. Third-party components use the same manifest format, which is how a default library and someone else's mix without collision. Rejected: everything in the crate (Askama templates have no stable cross-crate path; no editable markup) and everything vendored (lockstep behavior code outside the tree, the oauth crate's re-tag treadmill).

## Realized by

- f241aba8  docs(cairn,live): apply the owner's component-library rulings to the specs
