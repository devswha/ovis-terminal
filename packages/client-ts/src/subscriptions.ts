import type { Client } from "./client";
import type {
  EventCursor,
  EventEnvelope,
  EventFilter,
  EventsSubscribeResult,
  Heartbeat,
  MissedNotice,
  SubscriptionId,
  TerminalRecord,
} from "./generated/components";

export type SubscriptionState = "live" | "missed" | "detached";

/** Why delivery cannot continue from the retained cursor, as the server reported it. */
export interface Missed {
  readonly reason: string;
  readonly fromSeq: string | undefined;
  readonly snapshot: readonly TerminalRecord[] | undefined;
}

/**
 * An event subscription that keeps the cursor of the last delivered event. A disconnect
 * detaches it; `resume` resubscribes after that cursor on the client's current connection.
 */
export class Subscription {
  private stateNow: SubscriptionState = "live";
  private cursorNow: EventCursor | null;
  private idNow: SubscriptionId;
  private missedNow: Missed | undefined;
  private readonly delivered: EventEnvelope[] = [];
  private readonly waiters: ((event: EventEnvelope) => void)[] = [];
  staleDropped = 0;

  private constructor(
    private readonly client: Client,
    private readonly filter: EventFilter,
    opened: EventsSubscribeResult,
    cursor: EventCursor | null,
  ) {
    this.idNow = opened.subscription_id;
    this.cursorNow = cursor;
    this.adopt(opened);
    client.on("event", (params) => this.event(params as EventEnvelope));
    client.on("missed", (params) => this.missed(params as MissedNotice));
    client.on("heartbeat", (params) => this.heartbeat(params as Heartbeat));
    client.onDisconnect(() => {
      this.stateNow = "detached";
    });
  }

  static async open(client: Client, filter: EventFilter, cursor: EventCursor | null = null): Promise<Subscription> {
    const opened = await client.call("events.subscribe", { filter, snapshot: cursor === null, cursor });
    return new Subscription(client, filter, opened, cursor);
  }

  get id(): SubscriptionId {
    return this.idNow;
  }

  get state(): SubscriptionState {
    return this.stateNow;
  }

  get cursor(): EventCursor | null {
    return this.cursorNow;
  }

  get missedReport(): Missed | undefined {
    return this.missedNow;
  }

  /** Resubscribes after the retained cursor; events already delivered are never repeated. */
  async resume(): Promise<void> {
    const opened = await this.client.call("events.subscribe", { filter: this.filter, snapshot: false, cursor: this.cursorNow });
    this.idNow = opened.subscription_id;
    this.stateNow = "live";
    this.adopt(opened);
  }

  next(): Promise<EventEnvelope> {
    const ready = this.delivered.shift();
    if (ready !== undefined) return Promise.resolve(ready);
    return new Promise((resolve) => this.waiters.push(resolve));
  }

  private adopt(opened: EventsSubscribeResult): void {
    if (opened.missed !== undefined && opened.missed !== null) {
      this.stateNow = "missed";
      this.missedNow = { reason: opened.missed, fromSeq: undefined, snapshot: opened.snapshot ?? undefined };
      this.cursorNow = null;
    }
  }

  private event(envelope: EventEnvelope): void {
    if (envelope.subscription_id !== this.idNow || this.stateNow !== "live") return;
    const cursor = this.cursorNow;
    const stale = cursor !== null && cursor.runtime_lifetime === envelope.lifetime && BigInt(envelope.seq) <= BigInt(cursor.seq);
    if (stale) {
      this.staleDropped += 1;
      return;
    }
    this.cursorNow = { runtime_lifetime: envelope.lifetime, seq: envelope.seq };
    const waiter = this.waiters.shift();
    if (waiter === undefined) this.delivered.push(envelope);
    else waiter(envelope);
  }

  private missed(notice: MissedNotice): void {
    if (notice.subscription_id !== this.idNow) return;
    this.stateNow = "missed";
    this.missedNow = { reason: notice.reason, fromSeq: notice.from_seq, snapshot: undefined };
  }

  private heartbeat(beat: Heartbeat): void {
    if (beat.subscription_id === this.idNow && this.cursorNow === null) {
      this.cursorNow = { runtime_lifetime: beat.lifetime, seq: beat.seq };
    }
  }
}
