import type { ReactNode } from 'react';
import { useContent, useLang } from '@/i18n';
import type { SandboxRow } from '@/lib/api/operations';
import { ATTRIBUTION_SOURCES, SANDBOX_STATUSES, asMember } from '@/lib/api/operations';
import { formatLocal } from '@/lib/format';
import {
  elapsedSeconds,
  formatDurationSeconds,
  lifetimeDisplay,
  sandboxTone,
} from '@/lib/operations/format';
import { cn } from '@/lib/utils';
import { Absent, StatusPill, Truncated } from './parts';

/**
 * The live sandbox table.
 *
 * Three renderings here are the ones that must not be "simplified":
 *
 * - **`Unlimited` is not `0s`.** A `null` maximum lifetime means the deployment
 *   configured no ceiling. Rendering a countdown for it would tell an operator a
 *   runtime is about to be reaped when nothing will reap it.
 * - **`Not reported` is not `0`.** OpenSandbox exposes no restart concept, so a
 *   `null` restart count is the ABSENCE of a measurement. A zero would assert one
 *   was taken.
 * - **A missing creator is never guessed.** No repository owner, no trigger
 *   author, no "probably them" — the row shows the attribution state, which is
 *   the only thing actually known.
 *
 * Age and remaining time are recomputed from the display clock (`now`) so a
 * countdown keeps moving between 5-second polls, WITHOUT mutating any server
 * fact: the underlying instants are what the backend observed.
 */

const COLUMNS = [
  'w-[132px]', // status
  'w-[220px]', // session / runtime
  'w-[150px]', // creator
  'w-[188px]', // repository / issue
  'w-[152px]', // created
  'w-[92px]', // age
  'w-[168px]', // lifetime
  'w-[132px]', // idle
  'w-[92px]', // restarts
  'w-[200px]', // transition
] as const;

export function SandboxTable({
  rows,
  now,
  selectedId,
  onSelect,
}: {
  rows: SandboxRow[];
  /** The display clock. Injected so a test can pin it and so every row in one
   *  render agrees on "now". */
  now: number;
  selectedId: string | null;
  onSelect: (row: SandboxRow) => void;
}) {
  const t = useContent().operations;
  const { lang } = useLang();
  const headers = [
    t.colSandboxStatus,
    t.colSandboxId,
    t.colCreator,
    t.colRepository,
    t.colCreated,
    t.colAge,
    t.colLifetime,
    t.colIdle,
    t.colRestarts,
    t.colTransition,
  ];

  return (
    <table
      aria-label={t.sandboxTableAria}
      className="table-fixed border-collapse min-w-[1520px] w-full"
    >
      <thead className="sticky top-0 z-10">
        <tr>
          {headers.map((header, index) => (
            <th
              key={header}
              scope="col"
              className={cn(
                'text-left font-mono text-[9.5px] uppercase tracking-[0.14em] text-ghost',
                'bg-glass backdrop-blur-glass border-b border-line px-2 py-2 whitespace-nowrap',
                COLUMNS[index]
              )}
            >
              {header}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {rows.map((row) => {
          const status = asMember(SANDBOX_STATUSES, row.status);
          const attribution = asMember(ATTRIBUTION_SOURCES, row.attribution_source);
          const age = elapsedSeconds(row.created_at, now) ?? row.age_seconds;
          const idle = elapsedSeconds(row.last_pending_at, now) ?? row.idle_for_seconds;
          const lifetime = lifetimeDisplay(
            row.max_lifetime_seconds,
            row.expires_at,
            row.remaining_seconds,
            now
          );
          const rowId = rowKey(row);
          return (
            <tr
              key={rowId}
              data-testid="sandbox-row"
              data-selected={rowId === selectedId || undefined}
              tabIndex={0}
              role="button"
              aria-label={`${t.openDetails}: ${row.runtime_id}`}
              onClick={() => onSelect(row)}
              onKeyDown={(event) => {
                if (event.key !== 'Enter' && event.key !== ' ') return;
                event.preventDefault();
                onSelect(row);
              }}
              className={cn(
                'border-b border-line/60 cursor-pointer transition-colors',
                'hover:bg-raise focus-visible:outline focus-visible:outline-1 focus-visible:outline-amber',
                rowId === selectedId && 'bg-raise'
              )}
            >
              <Cell index={0}>
                <StatusPill tone={sandboxTone(row.status)} title={row.raw_status}>
                  {status ? t.sandboxStatus[status] : row.status}
                </StatusPill>
              </Cell>
              <Cell index={1}>
                <Truncated value={row.session_id} />
                <Truncated value={row.runtime_id} className="text-ghost text-[10px]" />
              </Cell>
              <Cell index={2}>
                {row.creator_login ? (
                  <Truncated value={`@${row.creator_login}`} />
                ) : row.creator_id !== null ? (
                  <Truncated value={`#${row.creator_id}`} />
                ) : (
                  // Never a guess: the attribution state IS the answer.
                  <span className="text-ghost">
                    {attribution ? t.attribution[attribution] : t.unknownCreator}
                  </span>
                )}
                {attribution === 'conflict' && (
                  <span className="block">
                    <StatusPill tone="red">{t.attribution.conflict}</StatusPill>
                  </span>
                )}
              </Cell>
              <Cell index={3}>
                <Truncated value={row.repo_full_name} />
                {row.trigger_issue !== null && (
                  <Truncated value={`#${row.trigger_issue}`} className="text-ghost text-[10px]" />
                )}
              </Cell>
              <Cell index={4}>
                {row.created_at ? (
                  <span title={row.created_at} className="tabular-nums">
                    {formatLocal(row.created_at, lang)}
                  </span>
                ) : (
                  <Absent />
                )}
              </Cell>
              <Cell index={5}>
                {age !== null ? (
                  <span className="tabular-nums">{formatDurationSeconds(age)}</span>
                ) : (
                  <Absent />
                )}
              </Cell>
              <Cell index={6}>
                {lifetime.kind === 'unlimited' ? (
                  // NOT "0s remaining": no ceiling was configured at all.
                  <span className="text-faint">{t.unlimited}</span>
                ) : (
                  <>
                    <span className="tabular-nums">
                      {formatDurationSeconds(lifetime.maxSeconds)}
                    </span>
                    <span className="block text-ghost text-[10px] tabular-nums">
                      {lifetime.remaining === null
                        ? t.notReported
                        : lifetime.remaining <= 0
                          ? t.expired
                          : t.remaining.replace(
                              '{duration}',
                              formatDurationSeconds(lifetime.remaining)
                            )}
                    </span>
                  </>
                )}
              </Cell>
              <Cell index={7}>
                {idle !== null ? (
                  <span className="tabular-nums">{formatDurationSeconds(idle)}</span>
                ) : (
                  <span className="text-ghost">{t.neverPending}</span>
                )}
              </Cell>
              <Cell index={8}>
                {row.restart_count === null ? (
                  // The backend has no restart concept; zero would be a lie.
                  <span className="text-ghost">{t.notReported}</span>
                ) : (
                  <span className="tabular-nums">{row.restart_count}</span>
                )}
              </Cell>
              <Cell index={9}>
                {row.status_reason ? (
                  <Truncated value={row.status_reason} />
                ) : row.last_transition_at ? (
                  <span title={row.last_transition_at} className="tabular-nums">
                    {formatLocal(row.last_transition_at, lang)}
                  </span>
                ) : (
                  <Absent />
                )}
              </Cell>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

/** A stable per-row identity. `runtime_id` alone can repeat across backends and
 *  namespaces; the pair cannot. */
export function rowKey(row: SandboxRow): string {
  return `${row.backend}:${row.backend_location ?? ''}:${row.runtime_id}`;
}

function Cell({ index, children }: { index: number; children: ReactNode }) {
  return (
    <td
      className={cn(
        'px-2 py-1.5 font-mono text-[11px] text-dim align-top overflow-hidden',
        COLUMNS[index]
      )}
    >
      {children}
    </td>
  );
}
