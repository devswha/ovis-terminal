import { ClientError } from "./errors";
import type { OpCancelResult, OpId, OpRecord } from "./generated/components";

/** The caller's later decision about a mutation whose outcome a lost reply hid. */
export type DraftDecision = "pending" | "retried" | "discarded";

export interface DraftActions {
  resend(method: string, params: unknown): Promise<unknown>;
  inspect(opId: OpId): Promise<OpRecord>;
  cancel(opId: OpId): Promise<OpCancelResult>;
}

/**
 * A mutation that may or may not have taken effect. It is never resent automatically;
 * `retry` reuses the same op_id so the server converges instead of repeating the effect.
 */
export class Draft {
  private state: DraftDecision = "pending";

  constructor(
    readonly method: string,
    readonly params: Readonly<Record<string, unknown>>,
    readonly opId: OpId,
    private readonly actions: DraftActions,
  ) {}

  get decision(): DraftDecision {
    return this.state;
  }

  inspect(): Promise<OpRecord> {
    return this.actions.inspect(this.opId);
  }

  retry(): Promise<unknown> {
    this.decide("retried");
    return this.actions.resend(this.method, this.params);
  }

  discard(): Promise<OpCancelResult> {
    this.decide("discarded");
    return this.actions.cancel(this.opId);
  }

  private decide(next: DraftDecision): void {
    if (this.state !== "pending") {
      throw new ClientError("draft_settled", `draft ${this.opId} was already ${this.state}`);
    }
    this.state = next;
  }
}

/** A call whose reply was lost after it may have been admitted; carries the retained draft. */
export class OutcomeUnknown extends ClientError {
  constructor(
    kind: "outcome_unknown" | "deadline_exceeded",
    readonly draft: Draft,
  ) {
    super(kind, `${draft.method} ${draft.opId}: no reply arrived; the outcome is unknown`);
    this.name = "OutcomeUnknown";
  }
}
