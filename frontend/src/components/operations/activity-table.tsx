import { useContent, useLang } from '@/i18n';
import type { ActivityRow, ApiRequestRow, SandboxLifecycleRow } from '@/lib/api/operations';
import {
  ACTOR_KINDS,
  DELIVERY_STATES,
  LIFECYCLE_ACTIONS,
  OUTCOMES,
  PRINCIPAL_KINDS,
  asMember,
} from '@/lib/api/operations';
import { formatAbsolute, formatLocal } from '@/lib/format';
import { deliveryTone, formatDurationMs, outcomeTone, summarizeArguments } from '@/lib/operations/format';
import { cn } from '@/lib/utils';
import { Absent, StatusPill, Truncated } from './parts';

/**
 * The Activity table.
 *
 * Its whole job is to render each row through ITS OWN contract. A lifecycle
 * transition has no HTTP method, no status code and no duration; those cells
 * render the absent dash rather than a plausible-looking zero, because a
 * fabricated `200` in an audit trail is worse than a blank.
 *
 * The layout is `table-fixed` with explicit column widths. That is not cosmetic:
 * a 15-second poll that reflowed columns whenever a longer operation id arrived
 * would make the table unreadable exactly when it matters.
 */

/** Column widths, in the header's declaration order. Fixed so a poll cannot
 *  resize anything. */
const COLUMNS = [
  'w-[152px]', // time
  'w-[92px]', // kind
  'w-[140px]', // actor
  'w-[128px]', // principal
  'w-[72px]', // method
  'w-[200px]', // operation
  'w-[220px]', // arguments
  'w-[128px]', // status
  'w-[84px]', // duration
  'w-[176px]', // correlation
  'w-[112px]', // delivery
  'w-[168px]', // request id
] as const;

export function ActivityTable({
  rows,
  selectedId,
  onSelect,
}: {
  rows: ActivityRow[];
  selectedId: string | null;
  onSelect: (row: ActivityRow) => void;
}) {
  const t = useContent().operations;
  const { lang } = useLang();
  const headers = [
    t.colTime,
    t.colRecordKind,
    t.colActor,
    t.colPrincipal,
    t.colMethod,
    t.colOperation,
    t.colArguments,
    t.colStatus,
    t.colDuration,
    t.colCorrelation,
    t.colDelivery,
    t.colRequestId,
  ];

  return (
    <table
      aria-label={t.activityTableAria}
      className="table-fixed border-collapse min-w-[1680px] w-full"
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
          const time = row.record_kind === 'api_request' ? row.completed_at : row.occurred_at;
          return (
            <tr
              key={row.event_id}
              data-testid="activity-row"
              data-selected={row.event_id === selectedId || undefined}
              // The row is a button-like target: click OR Enter/Space opens the
              // details panel, and it is reachable by keyboard in row order.
              tabIndex={0}
              role="button"
              aria-label={`${t.openDetails}: ${row.event_id}`}
              onClick={() => onSelect(row)}
              onKeyDown={(event) => {
                if (event.key !== 'Enter' && event.key !== ' ') return;
                event.preventDefault();
                onSelect(row);
              }}
              className={cn(
                'border-b border-line/60 cursor-pointer transition-colors',
                'hover:bg-raise focus-visible:outline focus-visible:outline-1 focus-visible:outline-amber',
                row.event_id === selectedId && 'bg-raise'
              )}
            >
              <Cell index={0}>
                <span title={time} className="tabular-nums">
                  {formatLocal(time, lang)}
                </span>
              </Cell>
              <Cell index={1}>
                <span className="text-faint">{t.recordKind[row.record_kind]}</span>
              </Cell>
              <Cell index={2}>{actorCell(row, t)}</Cell>
              <Cell index={3}>
                <Truncated
                  value={
                    asMember(PRINCIPAL_KINDS, row.principal.kind)
                      ? t.principalKind[asMember(PRINCIPAL_KINDS, row.principal.kind)!]
                      : (row.principal.kind ?? null)
                  }
                />
              </Cell>
              {row.record_kind === 'api_request'
                ? apiCells(row, t, lang)
                : lifecycleCells(row, t)}
              <Cell index={9}>{correlationCell(row)}</Cell>
              <Cell index={10}>
                <StatusPill tone={deliveryTone(row.delivery_state)} title={row.source}>
                  {asMember(DELIVERY_STATES, row.delivery_state)
                    ? t.delivery[asMember(DELIVERY_STATES, row.delivery_state)!]
                    : row.delivery_state}
                </StatusPill>
              </Cell>
              <Cell index={11}>
                <Truncated
                  value={
                    row.record_kind === 'api_request'
                      ? (row.request_id ?? row.correlation.request_id ?? null)
                      : (row.correlation.request_id ?? null)
                  }
                />
              </Cell>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

type Catalog = ReturnType<typeof useContent>['operations'];

function Cell({ index, children }: { index: number; children: React.ReactNode }) {
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

/** The actor cell prefers the login snapshot; the immutable id lives in the
 *  details. A row with neither is a system or anonymous actor, and says so by
 *  its KIND rather than by guessing a name. */
function actorCell(row: ActivityRow, t: Catalog) {
  const kind = asMember(ACTOR_KINDS, row.actor.kind);
  if (row.actor.login) {
    return <Truncated value={`@${row.actor.login}`} />;
  }
  if (kind) {
    return <span className="text-faint">{t.actorKind[kind]}</span>;
  }
  return <Absent />;
}

/** Method / operation / arguments / status / duration for an API request. */
function apiCells(row: ApiRequestRow, t: Catalog, lang: 'en' | 'zh') {
  const outcome = asMember(OUTCOMES, row.outcome);
  const duration = formatDurationMs(row.duration_ms);
  const summary = summarizeArguments(row.arguments);
  return (
    <>
      <Cell index={4}>
        <span className="text-faint tabular-nums">{row.method}</span>
      </Cell>
      <Cell index={5}>
        {/* The normalized route template, never a raw URI. */}
        <Truncated value={row.operation_id} />
        <Truncated value={row.route_template} className="text-ghost text-[10px]" />
      </Cell>
      <Cell index={6}>
        {summary ? (
          <Truncated value={summary} className="text-ghost" />
        ) : (
          <span className="text-ghost">{t.noArguments}</span>
        )}
      </Cell>
      <Cell index={7}>
        <span className="inline-flex items-center gap-1">
          <StatusPill
            tone={outcome ? outcomeTone(outcome) : 'neutral'}
            title={formatAbsolute(row.completed_at, lang)}
          >
            {row.status_code ?? '—'}
          </StatusPill>
          <span className="text-ghost text-[10px]">
            {outcome ? t.outcome[outcome] : row.outcome}
          </span>
        </span>
      </Cell>
      <Cell index={8}>
        {duration ? <span className="tabular-nums">{duration}</span> : <Absent />}
      </Cell>
    </>
  );
}

/** The same five columns for a lifecycle transition. Method, status and duration
 *  are deliberately absent: this record type has none, and inventing them would
 *  make the audit trail lie. */
function lifecycleCells(row: SandboxLifecycleRow, t: Catalog) {
  const action = asMember(LIFECYCLE_ACTIONS, row.lifecycle_action);
  return (
    <>
      <Cell index={4}>
        <Absent />
      </Cell>
      <Cell index={5}>
        <Truncated value={action ? t.lifecycleAction[action] : row.lifecycle_action} />
        <Truncated value={row.runtime_id ?? null} className="text-ghost text-[10px]" />
      </Cell>
      <Cell index={6}>
        {row.reason_code ? <Truncated value={row.reason_code} className="text-ghost" /> : <Absent />}
      </Cell>
      <Cell index={7}>
        <Absent />
      </Cell>
      <Cell index={8}>
        <Absent />
      </Cell>
    </>
  );
}

/** Session id, falling back to `owner/name#issue`. */
function correlationCell(row: ActivityRow) {
  const sessionId =
    row.record_kind === 'sandbox_lifecycle' ? row.session_id : row.correlation.session_id;
  if (sessionId) return <Truncated value={sessionId} />;
  const repo = row.correlation.repo_full_name;
  if (!repo) return <Absent />;
  const issue = row.correlation.trigger_issue;
  return <Truncated value={issue ? `${repo}#${issue}` : repo} />;
}
