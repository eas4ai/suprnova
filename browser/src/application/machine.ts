import { evaluateResponseEligibility } from "./eligibility.js";
import type { ApplicationEpoch, RecoveryDecision } from "./recovery.js";
import type {
  BrowserIslandAuthority,
  EligibilityDisposition,
  ResponseRequestAuthority,
  ValidatedCommittedResponse,
  ValidatedNavigationResponse,
  ValidatedResponse,
} from "./types.js";

export interface ApplicationPorts<Prepared> {
  beginApplication(response: ValidatedCommittedResponse): ApplicationEpoch | null;
  applicationCurrent(epoch: ApplicationEpoch): boolean;
  preflight(response: ValidatedCommittedResponse): Prepared;
  morph(prepared: Prepared, response: ValidatedCommittedResponse): void | Promise<void>;
  validateNoRender(response: ValidatedCommittedResponse): void;
  commit(response: ValidatedCommittedResponse): void;
  rollbackCommit(response: ValidatedCommittedResponse): void;
  completeApplication(epoch: ApplicationEpoch, response: ValidatedCommittedResponse): void;
  reconcile(response: ValidatedCommittedResponse): void;
  restoreFocus(response: ValidatedCommittedResponse): void;
  queueChildren(response: ValidatedCommittedResponse): void;
  reflectUrl(response: ValidatedCommittedResponse): void;
  dispatchEvents(response: ValidatedCommittedResponse): void;
  runEffects(response: ValidatedCommittedResponse): void | Promise<void>;
  settleFeedback(response: ValidatedResponse): void;
  navigate(response: ValidatedNavigationResponse | ValidatedResponse): void;
  retainDom(response: ValidatedResponse): void;
  recover(
    error: unknown,
    response: ValidatedCommittedResponse,
    epoch: ApplicationEpoch,
  ): RecoveryDecision;
  requestFreshIsland(response: ValidatedResponse): void;
  stopLive(response: ValidatedResponse): void;
  postCommitFailure(error: unknown, response: ValidatedCommittedResponse): void;
}

export type ApplicationDisposition =
  | EligibilityDisposition
  | "committed"
  | "navigated"
  | "rejected"
  | "fresh_render"
  | "disconnected"
  | "stale_application"
  | "fresh_island"
  | "stopped"
  | "post_commit_failure";

export interface ApplicationResult {
  readonly disposition: ApplicationDisposition;
}

function result(disposition: ApplicationDisposition): ApplicationResult {
  return Object.freeze({ disposition });
}

function recoveryResult(decision: RecoveryDecision): ApplicationResult {
  switch (decision.disposition) {
    case "request_fresh_render":
      return result("fresh_render");
    case "disconnect_island":
      return result("disconnected");
    case "ignored":
      return result("stale_application");
  }
}

export class ResponseApplicationMachine<Prepared> {
  readonly #ports: ApplicationPorts<Prepared>;

  constructor(ports: ApplicationPorts<Prepared>) {
    this.#ports = ports;
  }

  async apply(
    response: ValidatedResponse,
    island: BrowserIslandAuthority,
    request: ResponseRequestAuthority,
  ): Promise<ApplicationResult> {
    const eligibility = evaluateResponseEligibility(response, island, request);
    if (eligibility.disposition !== "accepted") return result(eligibility.disposition);

    if (response.kind === "navigation") {
      this.#ports.navigate(response);
      return result("navigated");
    }
    if (response.kind === "rejected" || response.kind === "recovery" || response.kind === "fatal") {
      return this.#applyRejected(response);
    }

    const epoch = this.#ports.beginApplication(response);
    if (epoch === null) return result("disconnected");

    try {
      if (response.render.kind === "html") {
        const prepared = this.#ports.preflight(response);
        await this.#ports.morph(prepared, response);
      } else {
        this.#ports.validateNoRender(response);
      }
      if (!this.#ports.applicationCurrent(epoch)) return result("stale_application");
    } catch (error: unknown) {
      return recoveryResult(this.#ports.recover(error, response, epoch));
    }

    try {
      this.#ports.commit(response);
      this.#ports.reconcile(response);
      this.#ports.restoreFocus(response);
      if (response.childDeliveries.length !== 0) this.#ports.queueChildren(response);
      if (response.reflectedUrl !== null) this.#ports.reflectUrl(response);
      this.#ports.dispatchEvents(response);
      if (!this.#ports.applicationCurrent(epoch)) throw new Error("application_epoch_stale");
    } catch (error: unknown) {
      try {
        this.#ports.rollbackCommit(response);
      } catch {
        // Recovery remains mandatory even when projection rollback is incomplete.
      }
      return recoveryResult(this.#ports.recover(error, response, epoch));
    }
    try {
      await this.#ports.runEffects(response);
    } catch (error: unknown) {
      this.#reportPostCommitFailure(error, response);
    }
    try {
      this.#ports.completeApplication(epoch, response);
    } catch (error: unknown) {
      this.#reportPostCommitFailure(error, response);
    }
    try {
      this.#ports.settleFeedback(response);
    } catch (error: unknown) {
      this.#reportPostCommitFailure(error, response);
    }
    return result("committed");
  }

  #reportPostCommitFailure(error: unknown, response: ValidatedCommittedResponse): void {
    try {
      this.#ports.postCommitFailure(error, response);
    } catch {
      // A diagnostic or extension port cannot disguise committed server authority.
    }
  }

  #applyRejected(
    response: Extract<ValidatedResponse, { kind: "rejected" | "recovery" | "fatal" }>,
  ): ApplicationResult {
    if (response.recovery === "navigate") {
      this.#ports.navigate(response);
      return result("navigated");
    }
    this.#ports.retainDom(response);
    if (response.outcome === "refresh_required") {
      this.#ports.requestFreshIsland(response);
      this.#ports.settleFeedback(response);
      return result("fresh_island");
    }
    if (response.outcome === "fatal") {
      this.#ports.stopLive(response);
      this.#ports.settleFeedback(response);
      return result("stopped");
    }
    this.#ports.settleFeedback(response);
    return result("rejected");
  }
}
