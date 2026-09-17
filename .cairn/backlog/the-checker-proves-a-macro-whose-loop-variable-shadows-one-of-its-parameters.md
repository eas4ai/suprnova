# The checker proves a macro whose loop variable shadows one of its parameters

Surfaced from: LIVE-025
Captured: 2026-09-17T11:53:47.602Z

Found 2026-09-17 in the adversarial review of Live and the component library before 2.1.0. crates/suprnova-live/src/checker/branch.rs passes a macro call's literal argument bindings into loop bodies unchanged (expand_loop), so `{{ name }}` inside `{% for name in names %}` is checked as the call-site literal. Reproduced with the checker and Askama 0.16: a macro bound(name) whose body is `{% for name in names %}<input live:model.blur="{{ name }}">{% endfor %}`, called with "query", is proved with no diagnostics, while the same loop over `other` reports dynamic_structure_unproved; Askama renders live:model.blur="email" and live:model.blur="role" from the loop's data. Match-arm bindings take the same path. Askama itself refuses a let that rebinds a macro parameter, so let does not reach this.
