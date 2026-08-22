export type ApplicationStep =
  | "navigate"
  | "preflight_morph"
  | "morph"
  | "validate_no_render"
  | "commit_snapshot_and_revision"
  | "reconcile_models_and_validation"
  | "restore_focus"
  | "queue_child_deliveries"
  | "reflect_url"
  | "dispatch_events"
  | "run_registered_effects"
  | "settle_feedback"
  | "retain_dom"
  | "request_fresh_render_without_replay"
  | "request_fresh_island"
  | "stop_live";

export interface ApplicationPlanInput {
  readonly protocol: 1 | 2;
  readonly outcome: "accepted" | "duplicate" | "rejected" | "refresh_required" | "fatal";
  readonly render: "redirect" | "navigated" | "html" | "no_render" | "none";
  readonly morph: "not_attempted" | "succeeded" | "failed_after_acceptance";
  readonly hasChildDeliveries: boolean;
  readonly hasReflectedUrl: boolean;
  readonly recovery:
    "retain_dom" | "retry" | "refresh_island" | "remount_island" | "navigate" | "stop" | null;
}

export function applicationPlan(
  render: "redirect" | "html" | "no_render",
  morph: "not_attempted" | "succeeded" | "failed_after_acceptance",
): readonly ApplicationStep[] {
  return applicationPlanV2({
    protocol: 1,
    outcome: "accepted",
    render,
    morph,
    hasChildDeliveries: false,
    hasReflectedUrl: false,
    recovery: render === "redirect" ? "navigate" : null,
  });
}

export function applicationPlanV2(input: ApplicationPlanInput): readonly ApplicationStep[] {
  if (input.render === "redirect" || input.render === "navigated") return ["navigate"];
  if (input.outcome === "rejected") return ["retain_dom", "settle_feedback"];
  if (input.outcome === "refresh_required") {
    return input.recovery === "navigate"
      ? ["navigate"]
      : ["retain_dom", "request_fresh_island", "settle_feedback"];
  }
  if (input.outcome === "fatal") {
    return input.recovery === "navigate"
      ? ["navigate"]
      : ["retain_dom", "stop_live", "settle_feedback"];
  }
  if (input.render === "html" && input.morph === "failed_after_acceptance") {
    return ["preflight_morph", "morph", "request_fresh_render_without_replay"];
  }

  const tail: ApplicationStep[] = [
    "commit_snapshot_and_revision",
    "reconcile_models_and_validation",
    "restore_focus",
  ];
  if (input.hasChildDeliveries) tail.push("queue_child_deliveries");
  if (input.hasReflectedUrl) tail.push("reflect_url");
  tail.push("dispatch_events", "run_registered_effects", "settle_feedback");

  if (input.render === "html") return ["preflight_morph", "morph", ...tail];
  if (input.render === "no_render") return ["validate_no_render", ...tail];
  return ["retain_dom", "settle_feedback"];
}
