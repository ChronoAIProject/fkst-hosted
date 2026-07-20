import { useContent } from '@/i18n';
import type { ObserveSnapshot, ObserveQueue } from '@/lib/api/types';
import { StaggerItem } from '@/components/ui/motion';
import { SectionLabel } from './parts';

/** Length of a value only if it is actually an array (the observe payload is
 *  raw engine JSON — every field is best-effort, so nothing is assumed). */
function arrayLen(value: unknown): number | null {
  return Array.isArray(value) ? value.length : null;
}

/** Numeric value only if the field is actually a finite number (untrusted JSON). */
function num(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

function QueueRow({ queue, scale }: { queue: ObserveQueue; scale: number }) {
  const t = useContent().dashboard.detail;
  const stats: Array<[string, number | undefined]> = [
    [t.queueDepth, queue.depth],
    [t.queuePending, queue.pending],
    [t.queueInFlight, queue.in_flight],
    [t.queueRetrying, queue.retrying],
  ];
  const depth = num(queue.depth);
  const inFlight = num(queue.in_flight);
  // Bar length is this queue's depth as a fraction of the busiest queue's depth
  // (the shared `scale`), so lengths are directly comparable row-to-row — a
  // handy "which queue is most backed up" read. The green segment is the
  // in-flight share of that depth (work actively moving through the backlog).
  const depthPct = depth != null && scale > 0 ? Math.min(100, (depth / scale) * 100) : 0;
  const inFlightPct =
    depth != null && depth > 0 && inFlight != null ? Math.min(100, (inFlight / depth) * 100) : 0;

  return (
    // Frosted glass chip: a translucent raise-2 fill + hairline lifts each queue
    // off the panel; the amber-tinted count values pop the live numbers, and the
    // backlog bar beneath gives the counts a shape.
    <div className="flex flex-col gap-1.5 rounded-chip bg-glass-2 border border-line px-2.5 py-1.5 min-w-0">
      <div className="flex items-center justify-between gap-3 min-w-0">
        <code className="font-mono text-[11.5px] text-fg truncate min-w-0">{queue.queue ?? '—'}</code>
        <div className="flex items-center gap-2 flex-none">
          {stats
            .filter(([, n]) => typeof n === 'number')
            .map(([label, n]) => (
              <span key={label} className="font-mono text-[10.5px] text-dim">
                {label} <span className="text-amber">{n}</span>
              </span>
            ))}
        </div>
      </div>
      {scale > 0 && (
        // Decorative backlog viz — the counts above carry the meaning, so the
        // bar is aria-hidden. Static widths (no motion) so it is inherently
        // reduced-motion-safe.
        <div aria-hidden="true" className="relative h-1.5 rounded-full bg-raise-2 overflow-hidden">
          <div
            className="absolute inset-y-0 left-0 rounded-full bg-grad-accent"
            style={{ width: `${depthPct}%` }}
          >
            <div
              className="absolute inset-y-0 left-0 rounded-full bg-green"
              style={{ width: `${inFlightPct}%` }}
            />
          </div>
        </div>
      )}
    </div>
  );
}

/** Renders whatever an observe snapshot happens to carry — queues (with their
 *  depth / pending / in-flight / retrying counters and a per-queue backlog bar),
 *  a pending-delivery count and a dead-letter count. Tolerant by construction:
 *  absent or wrongly-typed fields are simply skipped, and when nothing usable is
 *  present the caller's `empty` shows. */
export function ObserveView({ snapshot }: { snapshot: ObserveSnapshot }) {
  const t = useContent().dashboard.detail;
  const queues = Array.isArray(snapshot.queues) ? snapshot.queues : [];
  const deliveries = arrayLen(snapshot.deliveries);
  const deadLetters = arrayLen(snapshot.dead_letters);

  const nothing = queues.length === 0 && deliveries == null && deadLetters == null;
  if (nothing) return <p className="font-mono text-[11.5px] text-ghost">{t.liveEngineEmpty}</p>;

  // Shared bar scale: the busiest queue's depth. Zero when no queue reports a
  // depth, which suppresses the bars entirely (nothing to compare).
  const scale = queues.reduce((m, q) => {
    const d = num(q.depth);
    return d != null && d > m ? d : m;
  }, 0);

  return (
    <div className="flex flex-col gap-3">
      {queues.length > 0 && (
        <div className="flex flex-col gap-0.5">
          <SectionLabel>{t.queues}</SectionLabel>
          <div className="flex flex-col gap-1 mt-1">
            {queues.map((queue, i) => (
              // BUG B3: the observe payload is untrusted engine JSON, so two
              // queues can legitimately share a `queue` name. Keying on the name
              // alone would collide; always append the positional index so the
              // React key is unique regardless of duplicate names. The stagger
              // gives the rows a settle-in cadence as the snapshot reveals.
              <StaggerItem key={`${queue.queue ?? 'q'}-${i}`} index={i}>
                <QueueRow queue={queue} scale={scale} />
              </StaggerItem>
            ))}
          </div>
        </div>
      )}
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 font-mono text-[11px] text-dim">
        {deliveries != null && <span>{t.deliveries.replace('{n}', String(deliveries))}</span>}
        {deadLetters != null && deadLetters > 0 && (
          <span className="text-red">{t.deadLetters.replace('{n}', String(deadLetters))}</span>
        )}
      </div>
    </div>
  );
}
