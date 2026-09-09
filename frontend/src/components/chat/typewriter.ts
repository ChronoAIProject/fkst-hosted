/**
 * The typewriter reveal buffer.
 *
 * A model provider does not stream at a human reading rate: it emits whatever the
 * transport happened to flush, which is often a single 400-character paragraph after a
 * pause. Appending those straight to the transcript makes the answer BLINK into
 * existence, which reads as a page load rather than a reply — and after a tool call it
 * is common for an entire answer to land in one frame.
 *
 * So the transport's deltas are not what the user sees. They are pushed into this queue,
 * and the queue releases characters on a timer. The result is the same text at a steady,
 * legible rate no matter how the provider chunked it.
 *
 * Two properties matter and are why this is a class rather than a `useState` in the
 * provider:
 *
 * 1. **It never falls behind.** The per-tick reveal is computed from the CURRENT backlog,
 *    so a burst drains inside `drainWindowMs` instead of trickling out for a minute. The
 *    rate adapts; the smoothness does not.
 * 2. **It always terminates.** `finish` runs its callback when the queue drains,
 *    `flush` empties it immediately (the user pressed stop — they want what arrived, not
 *    an animation), and `cancel` abandons it. A turn cannot end with text stuck in a
 *    buffer.
 */

/** Reveal tick. ~60fps: fast enough to look continuous, slow enough to be cheap. */
const DEFAULT_INTERVAL_MS = 16;
/** How quickly a backlog should fully drain. The rate is derived from this and the
 *  backlog, so a big chunk speeds up rather than queueing for ages. */
const DEFAULT_DRAIN_WINDOW_MS = 700;
/** Floor on chars per tick, so even a 3-character backlog still moves. */
const MIN_CHARS_PER_TICK = 1;
/** Ceiling on chars per tick. A huge paste still animates rather than snapping in. */
const DEFAULT_MAX_CHARS_PER_TICK = 24;

export interface TypewriterOptions {
  intervalMs?: number;
  drainWindowMs?: number;
  maxCharsPerTick?: number;
  /** Reveal everything the moment it is pushed. Set for `prefers-reduced-motion`, where
   *  an animated reveal is the thing the user asked not to see. */
  instant?: boolean;
}

/** Characters to release this tick so `backlog` drains within the window. */
export function charsPerTick(
  backlog: number,
  intervalMs: number,
  drainWindowMs: number,
  maxCharsPerTick: number
): number {
  const ticksToDrain = Math.max(1, drainWindowMs / intervalMs);
  const needed = Math.ceil(backlog / ticksToDrain);
  return Math.min(maxCharsPerTick, Math.max(MIN_CHARS_PER_TICK, needed));
}

export class TypewriterQueue {
  private queued = '';
  private timer: ReturnType<typeof setInterval> | null = null;
  private onDrained: (() => void) | null = null;
  private readonly intervalMs: number;
  private readonly drainWindowMs: number;
  private readonly maxCharsPerTick: number;
  private readonly instant: boolean;

  /** @param reveal called with each released slice, in order. */
  constructor(
    private readonly reveal: (slice: string) => void,
    options: TypewriterOptions = {}
  ) {
    this.intervalMs = options.intervalMs ?? DEFAULT_INTERVAL_MS;
    this.drainWindowMs = options.drainWindowMs ?? DEFAULT_DRAIN_WINDOW_MS;
    this.maxCharsPerTick = options.maxCharsPerTick ?? DEFAULT_MAX_CHARS_PER_TICK;
    this.instant = options.instant ?? false;
  }

  /** Text still waiting to be shown. */
  get pending(): boolean {
    return this.queued.length > 0;
  }

  /** Accept a delta from the transport. */
  push(text: string): void {
    if (text === '') return;
    if (this.instant) {
      this.reveal(text);
      return;
    }
    this.queued += text;
    this.start();
  }

  /**
   * Signal end-of-stream: run `onDrained` once everything queued has been revealed.
   *
   * Deliberately NOT immediate. The turn is over on the wire, but it is not over for the
   * reader, and dropping the caret (or re-enabling the composer) while text is still
   * appearing would contradict what they are watching.
   */
  finish(onDrained: () => void): void {
    if (!this.pending) {
      onDrained();
      return;
    }
    this.onDrained = onDrained;
  }

  /** Reveal everything still queued, now, and run any pending completion. */
  flush(): void {
    const remaining = this.queued;
    this.queued = '';
    this.stopTimer();
    if (remaining !== '') this.reveal(remaining);
    this.runDrained();
  }

  /** Abandon the queue without revealing or completing. For an unmount or an abort. */
  cancel(): void {
    this.queued = '';
    this.onDrained = null;
    this.stopTimer();
  }

  private start(): void {
    if (this.timer != null) return;
    this.timer = setInterval(() => this.tick(), this.intervalMs);
  }

  private tick(): void {
    if (!this.pending) {
      this.stopTimer();
      this.runDrained();
      return;
    }
    const take = charsPerTick(
      this.queued.length,
      this.intervalMs,
      this.drainWindowMs,
      this.maxCharsPerTick
    );
    // Slice by code POINT, not code unit: cutting between a surrogate pair would render
    // a replacement character mid-word, and emoji do appear in issue titles and logs.
    const points = Array.from(this.queued);
    const slice = points.slice(0, take).join('');
    this.queued = points.slice(take).join('');
    this.reveal(slice);
    if (!this.pending) {
      this.stopTimer();
      this.runDrained();
    }
  }

  private runDrained(): void {
    const done = this.onDrained;
    this.onDrained = null;
    done?.();
  }

  private stopTimer(): void {
    if (this.timer != null) {
      clearInterval(this.timer);
      this.timer = null;
    }
  }
}

/** Whether the viewer asked for reduced motion. Read per turn, not cached, so a change
 *  in OS settings takes effect on the next question. */
export function prefersReducedMotion(): boolean {
  return (
    typeof window !== 'undefined' &&
    typeof window.matchMedia === 'function' &&
    window.matchMedia('(prefers-reduced-motion: reduce)').matches
  );
}
