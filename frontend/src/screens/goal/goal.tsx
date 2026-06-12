import { Link } from 'react-router-dom';
import { cn } from '@/lib/utils';
import { PostureChip } from '@/components/status/posture-chip';

export interface LifecycleEvent {
  name: string;
  timestamp?: string;
  body?: string;
  marker?: string;
  trustedBy?: string;
  isCurrent?: boolean;
  type?: 'approve' | 'converge' | 'now';
}

export interface GoalProps {
  goalId?: string;
  title?: string;
  state?: string;
  version?: string;
  headSha?: string;
  branch?: string;
  blocksGoalId?: string;
  lifecycleEvents?: LifecycleEvent[];
  deliveries?: {
    status: 'ACK' | 'LEASED';
    name: string;
    gen: number;
    state: string;
    timeLeft?: string;
    sourceRef?: string;
  }[];
  runs?: {
    exitCode: number | null;
    action: string;
    duration: string;
    permits?: number;
  }[];
  pr?: {
    number: number;
    href: string;
  };
  isReal?: boolean;
  mergeGate?: {
    reviewApproved?: 'ok' | 'fail' | 'unknown';
    headBound?: 'ok' | 'fail' | 'unknown';
    ciGreen?: 'ok' | 'fail' | 'unknown';
    mergeable?: 'ok' | 'fail' | 'unknown';
    posture?: 'ok' | 'fail' | 'unknown';
  };
  consensus?: {
    summary: string;
    passes?: boolean;
  };
}

export function Goal({
  goalId = '—',
  title,
  state = 'unknown',
  version = 'unknown',
  headSha = 'unknown',
  branch = 'unknown',
  blocksGoalId,
  lifecycleEvents = [],
  deliveries = [],
  runs = [],
  pr,
  isReal = false,
  mergeGate,
  consensus,
}: GoalProps) {
  const hasData = title !== undefined || lifecycleEvents.length > 0;

  return (
    <div className="flex flex-col gap-6">
      {/* Back navigation row */}
      <div className="flex items-center justify-between gap-4 flex-wrap">
        <Link
          to="/goals"
          className="font-mono text-[11.5px] text-ghost no-underline hover:text-dim transition-colors"
        >
          ← Goals · list
        </Link>
        <button
          disabled
          className="font-ui font-semibold text-[12.5px] bg-amber/50 text-amber-ink/50 cursor-not-allowed rounded-control px-3.5 py-[7px] opacity-50 select-none flex-shrink-0"
        >
          + New goal
        </button>
      </div>

      {/* Decision Header */}
      <div className="pb-[22px] border-b border-line">
        <div className="flex items-baseline gap-[11px] mt-2.5 mb-3 flex-wrap">
          {title ? (
            <h1 className="font-display font-semibold text-[23px] tracking-[-0.01em] text-fg leading-[1.1]">
              {title}
            </h1>
          ) : (
            <span className="font-display font-semibold text-[23px] tracking-[-0.01em] text-ghost select-none">
              —
            </span>
          )}
          <span className="font-mono text-[14px] text-faint select-none">
            #{goalId}
            {pr && (
              <>
                {' · '}
                <a
                  href={pr.href}
                  target="_blank"
                  rel="noreferrer"
                  className="text-faint hover:text-dim hover:underline"
                >
                  PR #{pr.number}
                </a>
              </>
            )}
          </span>
        </div>

        <div className="flex items-center gap-3.5 flex-wrap text-dim text-[12px]">
          {/* State Pill */}
          <div className="inline-flex items-center gap-[7px] font-ui font-semibold text-[11.5px] tracking-[0.02em] uppercase px-[10px] py-[5px] rounded-[8px] border border-line-2 text-dim bg-raise select-none">
            <span className={cn(
              "w-1.5 h-1.5 rounded-full",
              state === 'merged' && "bg-green",
              (state === 'blocked' || state === 'impl-failed') && "bg-red",
              (state === 'reviewing' || state === 'fixing') && "bg-gold",
              state === 'unknown' && "bg-ghost",
              state !== 'merged' && state !== 'blocked' && state !== 'impl-failed' && state !== 'reviewing' && state !== 'fixing' && state !== 'unknown' && "bg-faint"
            )} />
            <span>{state}</span>
          </div>

          <span className="font-mono text-ghost text-[11.5px]">= max trusted <b className="text-faint font-medium">state:v1</b> · labels are hints</span>
          <span className="font-mono text-ghost text-[11.5px]">version <b className="text-faint font-medium">{version}</b></span>
          <span className="font-mono text-ghost text-[11.5px]">head <b className="text-faint font-medium">{headSha}</b></span>
          <span className="font-mono text-ghost text-[11.5px]">branch <b className="text-faint font-medium">{branch}</b></span>
          <span className="font-mono text-ghost text-[11.5px]">
            blocks <b className="text-faint font-medium">{blocksGoalId ? `#${blocksGoalId}` : '—'}</b>
          </span>
        </div>

        {/* Inert Decide Box if populated, or omitted by default */}
        {isReal && (
          <div className="mt-[18px] flex items-center gap-[18px] bg-raise border border-line border-l-[3px] border-l-red rounded-[12px] p-[15px_18px] max-[780px]:flex-wrap">
            <span className="font-ui font-semibold text-[11px] tracking-[0.06em] uppercase text-red flex-shrink-0">
              Real · autonomous
            </span>
            <div className="flex-1 min-w-0">
              <div className="font-semibold text-[15px] text-fg">
                Merges PR into the <span className="font-mono">integration</span> branch — autonomously
              </div>
              <div className="font-mono text-[11.5px] text-ghost mt-[3px] leading-relaxed">
                REAL posture · CI green · review approved · head-bound. No per-goal pause exists.
              </div>
            </div>
            <button disabled className="font-ui font-medium text-[13px] border border-line-2 bg-raise-2 text-dim rounded-control px-4 py-[9px] cursor-not-allowed opacity-50 flex-shrink-0">
              Global write → DRY-RUN
            </button>
          </div>
        )}
      </div>

      {/* Grid Layout */}
      <div className="grid grid-cols-[1fr_348px] max-[980px]:grid-cols-1 gap-[34px] items-start pb-10">
        {/* Left Side: Lifecycle Timeline */}
        <div className="min-w-0">
          <div className="font-mono text-eyebrow text-ghost uppercase mb-4">
            Lifecycle · current state = MAX trusted state:v1 marker by (version, stage_rank) · fkst-dev labels are hints
          </div>

          {hasData && lifecycleEvents.length > 0 ? (
            <div className="tl relative mt-4 pl-[26px] before:content-[''] before:absolute before:left-1.5 before:top-1 before:bottom-2 before:w-[1px] before:bg-line-2">
              {lifecycleEvents.map((ev, idx) => (
                <div key={idx} className={cn("ev relative mb-5", ev.isCurrent ? "font-semibold" : "")}>
                  <span className={cn(
                    "node absolute left-[-26px] top-[3px] w-[11px] h-[11px] rounded-full border-2 border-bg",
                    ev.isCurrent ? "bg-amber" : "bg-line-2",
                    ev.type === 'approve' && "bg-green",
                    ev.type === 'converge' && "bg-gold"
                  )} />
                  <div className="hd flex items-center justify-between gap-[9px]">
                    <div className="flex items-center gap-2">
                      <span className="nm font-semibold text-[13.5px] text-fg">{ev.name}</span>
                      {ev.type && (
                        <span className={cn(
                          "chip font-mono text-[10px] px-1.5 py-0.5 rounded-[5px] border",
                          ev.type === 'approve' && "text-green border-[color-mix(in_oklab,var(--green)_40%,var(--line))]",
                          ev.type === 'converge' && "text-gold border-[color-mix(in_oklab,var(--gold)_40%,var(--line))]"
                        )}>
                          {ev.type}
                        </span>
                      )}
                      {ev.isCurrent && (
                        <span className="now font-mono text-[9.5px] text-red border border-[color-mix(in_oklab,var(--red)_45%,var(--line))] rounded-[5px] px-1.5 py-0.5">
                          ● NOW{isReal ? ' · REAL' : ''}
                        </span>
                      )}
                    </div>
                    {ev.timestamp && <span className="ts font-mono text-[11px] text-ghost">{ev.timestamp}</span>}
                  </div>
                  {ev.body && <div className="body font-mono text-[11.5px] text-dim mt-1">{ev.body}</div>}
                  {ev.marker && (
                    <div className="marker font-mono text-[10.5px] text-ghost bg-bg border border-line rounded-[7px] p-[8px_10px] mt-2 break-all select-all">
                      {ev.marker}
                    </div>
                  )}
                  {ev.trustedBy && (
                    <div className="trust font-mono text-[10px] text-faint mt-1.5">
                      <span className="text-green">✓ {ev.trustedBy}</span> · trusted (matches <span className="font-mono text-[11px]">FKST_GITHUB_BOT_LOGIN</span>)
                    </div>
                  )}
                </div>
              ))}
            </div>
          ) : (
            <div className="flex items-center justify-center py-12 text-ghost font-mono text-[12px] border border-dashed border-line rounded-panel bg-raise/20">
              no GitHub plane connected — sign-in pending
            </div>
          )}
        </div>

        {/* Right Side: Side Panels (Diagnostics & Merge Gate) */}
        <div className="flex flex-col gap-4">
          {/* Merge Gate Panel */}
          <div className="panel bg-raise border border-line rounded-[13px] overflow-hidden">
            <div className="ph p-[11px_14px] border-b border-line font-mono font-semibold text-[11px] tracking-[0.1em] uppercase text-faint flex items-center justify-between">
              Merge gate
              {mergeGate ? (
                <span className={cn(
                  "text-[10.5px] font-mono lowercase",
                  Object.values(mergeGate).every(v => v === 'ok') ? "text-green" : "text-ghost"
                )}>
                  {Object.values(mergeGate).every(v => v === 'ok') ? 'passing' : 'blocked'}
                </span>
              ) : (
                <span className="text-ghost text-[10px] lowercase">not at the gate yet</span>
              )}
            </div>
            <div className="pc p-[13px_14px] flex flex-col gap-[11px]">
              {mergeGate ? (
                <>
                  <div className="gate flex items-center gap-[10px] text-[12.5px] text-dim">
                    {mergeGate.reviewApproved === 'ok' ? (
                      <span className="mk ok text-green font-mono w-3.5 text-center">✓</span>
                    ) : mergeGate.reviewApproved === 'fail' ? (
                      <span className="mk fail text-red font-mono w-3.5 text-center">✗</span>
                    ) : (
                      <span className="mk text-ghost font-mono w-3.5 text-center">—</span>
                    )}
                    <span>trusted <span className="font-mono text-[11px] text-dim">review-result:v1 approve</span></span>
                  </div>
                  <div className="gate flex items-center gap-[10px] text-[12.5px] text-dim">
                    {mergeGate.headBound === 'ok' ? (
                      <span className="mk ok text-green font-mono w-3.5 text-center">✓</span>
                    ) : mergeGate.headBound === 'fail' ? (
                      <span className="mk fail text-red font-mono w-3.5 text-center">✗</span>
                    ) : (
                      <span className="mk text-ghost font-mono w-3.5 text-center">—</span>
                    )}
                    <span>head-bound <span className="font-mono text-[11px] text-dim">merge-ready:v1</span></span>
                  </div>
                  <div className="gate flex items-center gap-[10px] text-[12.5px] text-dim">
                    {mergeGate.ciGreen === 'ok' ? (
                      <span className="mk ok text-green font-mono w-3.5 text-center">✓</span>
                    ) : mergeGate.ciGreen === 'fail' ? (
                      <span className="mk fail text-red font-mono w-3.5 text-center">✗</span>
                    ) : (
                      <span className="mk text-ghost font-mono w-3.5 text-center">—</span>
                    )}
                    <span>CI green <span className="font-mono text-[11px] text-ghost">statusCheckRollup</span></span>
                  </div>
                  <div className="gate flex items-center gap-[10px] text-[12.5px] text-dim">
                    {mergeGate.mergeable === 'ok' ? (
                      <span className="mk ok text-green font-mono w-3.5 text-center">✓</span>
                    ) : mergeGate.mergeable === 'fail' ? (
                      <span className="mk fail text-red font-mono w-3.5 text-center">✗</span>
                    ) : (
                      <span className="mk text-ghost font-mono w-3.5 text-center">—</span>
                    )}
                    <span>mergeable · same-repo head</span>
                  </div>
                  <div className="gate flex items-center gap-[10px] text-[12.5px] text-dim">
                    {mergeGate.posture === 'ok' ? (
                      <span className="mk ok text-green font-mono w-3.5 text-center">✓</span>
                    ) : mergeGate.posture === 'fail' ? (
                      <span className="mk fail text-red font-mono w-3.5 text-center">✗</span>
                    ) : (
                      <span className="mk text-ghost font-mono w-3.5 text-center">—</span>
                    )}
                    <span className="flex items-center gap-1.5">
                      FKST_GITHUB_WRITE = <PostureChip />
                    </span>
                  </div>
                </>
              ) : (
                <>
                  <div className="gate flex items-center gap-[10px] text-[12.5px] text-dim select-none opacity-50">
                    <span className="mk text-ghost font-mono w-3.5 text-center">—</span>
                    <span>CI status unknown</span>
                  </div>
                  <div className="gate flex items-center gap-[10px] text-[12.5px] text-dim select-none opacity-50">
                    <span className="mk text-ghost font-mono w-3.5 text-center">—</span>
                    <span>Review approval pending</span>
                  </div>
                  <div className="gate flex items-center gap-[10px] text-[12.5px] text-dim select-none font-mono">
                    <span className="text-ghost w-3.5 text-center">—</span>
                    <span className="flex items-center gap-1.5">
                      Posture: <PostureChip />
                    </span>
                  </div>
                </>
              )}
            </div>
          </div>

          {/* Deliveries · redb */}
          <div className={cn(
            "panel bg-raise border border-line rounded-[13px] overflow-hidden",
            deliveries.length === 0 && "opacity-50"
          )}>
            <div className="ph p-[11px_14px] border-b border-line font-mono font-semibold text-[11px] tracking-[0.1em] uppercase text-faint flex items-center justify-between">
              Deliveries · redb
            </div>
            <div className="pc p-[13px_14px] flex flex-col gap-[11px]">
              {deliveries.length > 0 ? (
                deliveries.map((d, i) => (
                  <div key={i} className="flex flex-col gap-1 text-dim">
                    <div className="drow flex items-center gap-[9px] font-mono text-[11px]">
                      <span className={cn(
                        "st font-mono text-[9.5px] px-[7px] py-[2px] rounded-[5px] border",
                        d.status === 'ACK' ? "text-green border-[color-mix(in_oklab,var(--green)_40%,var(--line))]" : "text-amber border-[color-mix(in_oklab,var(--amber)_40%,var(--line))]"
                      )}>
                        {d.status}
                      </span>
                      <span>{d.name} · gen {d.gen} · {d.state}</span>
                    </div>
                    {d.timeLeft && <div className="diag font-mono text-[10px] text-ghost pl-[48px]">timeLeft: {d.timeLeft}</div>}
                    {d.sourceRef && <div className="diag font-mono text-[10px] text-ghost pl-[48px]">{d.sourceRef}</div>}
                  </div>
                ))
              ) : (
                <div className="flex flex-col items-center justify-center text-center py-6 text-ghost font-mono text-[11px]">
                  host telemetry not connected
                </div>
              )}
            </div>
          </div>

          {/* Runs · codex */}
          <div className={cn(
            "panel bg-raise border border-line rounded-[13px] overflow-hidden",
            runs.length === 0 && "opacity-50"
          )}>
            <div className="ph p-[11px_14px] border-b border-line font-mono font-semibold text-[11px] tracking-[0.1em] uppercase text-faint flex items-center justify-between">
              Runs · codex <span className="text-[10px] lowercase text-ghost font-normal ml-2">diagnostic</span>
            </div>
            <div className="pc p-[13px_14px] flex flex-col gap-[11px]">
              {runs.length > 0 ? (
                runs.map((r, i) => (
                  <div key={i} className="run flex items-center gap-[9px] font-mono text-[11px] text-dim">
                    <span className={r.exitCode === 0 ? "e text-green" : "text-red"}>
                      exit {r.exitCode === null ? '—' : r.exitCode}
                    </span>
                    <span>
                      {r.action} · {r.duration}
                      {r.permits && ` · permits ${r.permits}`}
                    </span>
                  </div>
                ))
              ) : (
                <div className="flex flex-col items-center justify-center text-center py-6 text-ghost font-mono text-[11px]">
                  host telemetry not connected
                </div>
              )}
            </div>
          </div>

          {/* Consensus · review */}
          <div className={cn(
            "panel bg-raise border border-line rounded-[13px] overflow-hidden",
            !consensus && "opacity-50"
          )}>
            <div className="ph p-[11px_14px] border-b border-line font-mono font-semibold text-[11px] tracking-[0.1em] uppercase text-faint flex items-center justify-between">
              Review · consensus <span className="text-[10px] lowercase text-ghost font-normal ml-2">diagnostic</span>
            </div>
            <div className="pc p-[13px_14px] flex flex-col gap-[11px]">
              {consensus ? (
                <div className="flex flex-col gap-2">
                  <div className="flex items-center gap-[9px] text-[12.5px] text-dim">
                    {consensus.passes ? (
                      <span className="text-green font-mono w-3.5 text-center">✓</span>
                    ) : (
                      <span className="text-red font-mono w-3.5 text-center">✗</span>
                    )}
                    <span className="font-semibold text-fg">consensus {consensus.passes ? 'passed' : 'failed'}</span>
                  </div>
                  <div className="font-mono text-[11px] text-ghost leading-relaxed">
                    {consensus.summary}
                  </div>
                </div>
              ) : (
                <div className="flex flex-col items-center justify-center text-center py-6 text-ghost font-mono text-[11px]">
                  host telemetry not connected
                </div>
              )}
            </div>
          </div>
        </div>
      </div>

      {/* Footer */}
      <div className="foot flex gap-6 font-mono text-[11px] text-ghost flex-wrap mt-6 pt-[14px] border-t border-line">
        <span>state as of <b>unknown</b> · merge gate from redb <b>unknown</b> · poll-derived</span>
        <span>every transition re-derived from GitHub on each tick</span>
      </div>
    </div>
  );
}
