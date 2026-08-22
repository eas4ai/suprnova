import { evaluateResponseEligibility } from "./eligibility.js";
import type {
  BrowserIslandAuthority,
  EligibilityDisposition,
  ResponseRequestAuthority,
  ValidatedCommittedResponse,
  ValidatedNavigationResponse,
  ValidatedResponse,
} from "./types.js";

export interface ApplicationPorts<Prepared> {
  preflight(response: ValidatedCommittedResponse): Prepared;
  morph(prepared: Prepared, response: ValidatedCommittedResponse): void | Promise<void>;
  validateNoRender(response: ValidatedCommittedResponse): void;
  commit(response: ValidatedCommittedResponse): void;
  reconcile(response: ValidatedCommittedResponse): void;
  restoreFocus(response: ValidatedCommittedResponse): void;
  queueChildren(response: ValidatedCommittedResponse): void;
  reflectUrl(response: ValidatedCommittedResponse): void;
  dispatchEvents(response: ValidatedCommittedResponse): void;
  runEffects(response: ValidatedCommittedResponse): void | Promise<void>;
  settleFeedback(response: ValidatedResponse): void;
  navigate(response: ValidatedNavigationResponse | ValidatedResponse): void;
  retainDom(response: ValidatedResponse): void;
  requestFreshRender(response: ValidatedCommittedResponse): void;
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
  | "fresh_island"
  | "stopped"
  | "post_commit_failure";

export interface ApplicationResult {
  readonly disposition: ApplicationDisposition;
}

function result(disposition: ApplicationDisposition): ApplicationResult {
  return Object.freeze({ disposition });
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

    try {
      if (response.render.kind === "html") {
        const prepared = this.#ports.preflight(response);
        await this.#ports.morph(prepared, response);
      } else {
        this.#ports.validateNoRender(response);
      }
    } catch {
      this.#ports.requestFreshRender(response);
      return result("fresh_render");
    }

    try {
      this.#ports.commit(response);
    } catch {
      this.#ports.requestFreshRender(response);
      return result("fresh_render");
    }
    try {
      this.#ports.reconcile(response);
      this.#ports.restoreFocus(response);
      if (response.childDeliveries.length !== 0) this.#ports.queueChildren(response);
      if (response.reflectedUrl !== null) this.#ports.reflectUrl(response);
      this.#ports.dispatchEvents(response);
      await this.#ports.runEffects(response);
      this.#ports.settleFeedback(response);
      return result("committed");
    } catch (error: unknown) {
      this.#ports.postCommitFailure(error, response);
      return result("post_commit_failure");
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
