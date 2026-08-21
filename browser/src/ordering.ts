export type ApplicationStep =
  | "navigate"
  | "preflight_morph"
  | "morph"
  | "validate_no_render"
  | "commit_snapshot_and_revision"
  | "reconcile_models_and_validation"
  | "restore_focus"
  | "dispatch_events"
  | "run_registered_effects"
  | "settle_feedback"
  | "request_fresh_render_without_replay";

export function applicationPlan(
  render: "redirect" | "html" | "no_render",
  morph: "not_attempted" | "succeeded" | "failed_after_acceptance",
): readonly ApplicationStep[] {
  if (render === "redirect") return ["navigate"];
  if (render === "html" && morph === "failed_after_acceptance") {
    return ["preflight_morph", "morph", "request_fresh_render_without_replay"];
  }
  const tail: readonly ApplicationStep[] = [
    "commit_snapshot_and_revision",
    "reconcile_models_and_validation",
    "restore_focus",
    "dispatch_events",
    "run_registered_effects",
    "settle_feedback",
  ];
  return render === "html"
    ? ["preflight_morph", "morph", ...tail]
    : ["validate_no_render", ...tail];
}
