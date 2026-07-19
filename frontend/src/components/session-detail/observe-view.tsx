import { useContent } from '@/i18n';
import type { ObserveSnapshot, ObserveQueue } from '@/lib/api/types';
import { StaggerItem } from '@/components/ui/motion';
import { SectionLabel } from './parts';

/** Length of a value only if it is actually an array (the observe payload is
 *  raw engine JSON — every field is best-effort, so nothing is assumed). */
function arrayLen(value: unknown): number | null {
  return Array.isArray(value) ? value.length : null;
}

function QueueRow({ queue }: { queue: ObserveQueue }) {
  const t = useContent().dashboard.detail;
  const stats: Array<[string, number | undefined]> = [
    [t.queueDepth, queue.depth],
    [t.queuePending, queue.pending],
    [t.queueInFlight, queue.in_flight],
    [t.queueRetrying, queue.retrying],
  ];
  return (
    <div className="flex items-center justify-between gap-3 py-1.5 min-w-0">
      <code className="font-mono text-[11.5px] text-fg truncate min-w-0">{queue.queue ?? '—'}</code>
      <div className="flex items-center gap-2 flex-none">
        {stats
          .filter(([, n]) => typeof n === 'number')
          .map(([label, n]) => (
            <span key={label} className="font-mono text-[10.5px] text-dim">
              {label} <span className="text-fg">{n}</span>
            </span>
          ))}
      </div>
    </div>
  );
}

/** Renders whatever an observe snapshot happens to carry — queues (with their
 *  depth / pending / in-flight / retrying counters), a pending-delivery count and
 *  a dead-letter count. Tolerant by construction: absent or wrongly-typed fields
 *  are simply skipped, and when nothing usable is present the caller's `empty`
 *  shows. */
export function ObserveView({ snapshot }: { snapshot: ObserveSnapshot }) {
  const t = useContent().dashboard.detail;
  const queues = Array.isArray(snapshot.queues) ? snapshot.queues : [];
  const deliveries = arrayLen(snapshot.deliveries);
  const deadLetters = arrayLen(snapshot.dead_letters);

  const nothing = queues.length === 0 && deliveries == null && deadLetters == null;
  if (nothing) return <p className="font-mono text-[11.5px] text-ghost">{t.liveEngineEmpty}</p>;

  return (
    <div className="flex flex-col gap-3">
      {queues.length > 0 && (
        <div className="flex flex-col gap-0.5">
          <SectionLabel>{t.queues}</SectionLabel>
          <div className="divide-y divide-[color-mix(in_oklab,var(--line)_55%,transparent)]">
            {queues.map((queue, i) => (
              // BUG B3: the observe payload is untrusted engine JSON, so two
              // queues can legitimately share a `queue` name. Keying on the name
              // alone would collide; always append the positional index so the
              // React key is unique regardless of duplicate names. The stagger
              // gives the rows a settle-in cadence as the snapshot reveals.
              <StaggerItem key={`${queue.queue ?? 'q'}-${i}`} index={i}>
                <QueueRow queue={queue} />
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
