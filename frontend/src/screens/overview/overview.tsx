import { useState } from 'react';
import { cn } from '@/lib/utils';
import { WindowControl } from '@/components/layout/window-control';
import { ViewSwitch } from '@/components/layout/view-switch';
import { VitalsCell } from '@/components/status/vitals-cell';
import { GoalView } from '@/lib/api/goals';
import { goalStatusPresentation } from '@/lib/api/goal-status';

export interface OverviewGoal {
  id: string;
  title: string;
  stage: 'Design' | 'Build' | 'Review' | 'Ship' | 'Blocked' | 'Merged';
  state: string;
  age: string;
  repo?: string;
  pr?: string;
  ci?: 'passing' | 'failing' | 'unknown';
  pressure?: boolean;
  gated?: boolean;
}

export interface OverviewProps {
  goals?: (OverviewGoal | GoalView)[];
  statusPresentation?: typeof goalStatusPresentation;
  vitals?: {
    inFlight?: number | 'unknown';
    merged24h?: number | 'unknown';
    deadEnded?: number | 'unknown';
    throughput?: string | 'unknown';
    medianReviewTime?: string | 'unknown';
    windowStart?: string;
    windowEnd?: string;
  };
  needsYou?: {
    lead: string;
    leadTone?: 'red' | 'gold';
    title: string;
    id: string;
    pr?: string;
    why: string;
    actionLabel: string;
    actionTone?: 'red';
  }[];
  initialView?: 'pipeline' | 'board';
  initialWindow?: string;

  // Wave-3 custom/optional props
  stageIo?: Record<
    string,
    { inCount: number | string | 'unknown'; outCount: number | string | 'unknown' }
  >;
  stageMore?: Record<string, string>;
  intakeDetails?: string[];
  mergedDetails?: string[];
  reviewPressureLabel?: string;
  shipTag?: string;
  onNewGoal?: () => void;
}

export function Overview({
  goals = [],
  vitals,
  needsYou,
  initialView = 'pipeline',
  initialWindow = '24h',
  stageIo,
  stageMore,
  intakeDetails = ['github-proxy', 'cron · 5m', 'unknown'],
  mergedDetails = ['— · 24h', 'terminal'],
  reviewPressureLabel,
  shipTag,
  onNewGoal,
}: OverviewProps) {
  const [view, setView] = useState<'pipeline' | 'board'>(initialView);
  const [timeWindow, setTimeWindow] = useState<string>(initialWindow);

  // Derive stage counts
  const getStageGoals = (stageName: string): OverviewGoal[] => {
    return (goals || []).filter((g): g is OverviewGoal => {
      if ('status' in g && !('stage' in g)) {
        return false;
      }
      return g.stage === stageName;
    });
  };
  const designGoals = getStageGoals('Design');
  const buildGoals = getStageGoals('Build');
  const reviewGoals = getStageGoals('Review');
  const shipGoals = getStageGoals('Ship');
  const mergedGoals = getStageGoals('Merged');

  const showData = goals.length > 0;

  // Resolve Stage I/O labels
  const getStageIo = (stageName: string) => {
    return stageIo?.[stageName] ?? { inCount: '—', outCount: '—' };
  };

  const designIo = getStageIo('Design');
  const buildIo = getStageIo('Build');
  const reviewIo = getStageIo('Review');
  const shipIo = getStageIo('Ship');

  return (
    <div className="flex flex-col gap-6">
      {/* Toolbar */}
      <div className="flex items-center gap-4 flex-wrap pb-3.5 border-b border-line">
        <button
          onClick={onNewGoal}
          disabled={!onNewGoal}
          className={cn(
            "font-ui font-semibold text-[12.5px] rounded-control px-3.5 py-[7px] flex-shrink-0 transition-colors",
            onNewGoal
              ? "bg-amber text-amber-ink hover:brightness-[1.06] cursor-pointer"
              : "bg-amber/50 text-amber-ink/50 cursor-not-allowed opacity-50 select-none"
          )}
        >
          + New goal
        </button>

        <h2 className="font-mono text-eyebrow text-ghost uppercase">Window</h2>
        <WindowControl value={timeWindow} onChange={setTimeWindow} />

        <div className="flex items-center gap-2 text-[13px] text-dim border border-line rounded-control px-[11px] py-1.5 bg-raise cursor-not-allowed select-none opacity-50">
          scope <span className="font-mono text-fg">all deployments</span> <span className="text-ghost text-[10px]">▾</span>
        </div>

        <div className="flex items-center gap-[9px] text-[13px] text-dim cursor-not-allowed select-none opacity-50">
          <span>Needs attention only</span>
          <div className="w-[30px] h-[18px] rounded-[10px] bg-line-2 relative transition-colors">
            <i className="absolute top-[2px] left-[2px] w-[14px] h-[14px] rounded-full bg-faint" />
          </div>
        </div>

        <ViewSwitch value={view} onChange={setView} className="ml-auto" />
      </div>

      {/* Vitals Panel */}
      <div className="border border-line rounded-panel overflow-hidden grid grid-cols-6 max-[980px]:grid-cols-3 max-[780px]:grid-cols-2 max-[480px]:grid-cols-2 gap-px bg-line">
        <VitalsCell value={vitals?.inFlight ?? 'unknown'} label="in flight now" />
        <VitalsCell value={vitals?.merged24h ?? 'unknown'} label="merged · 24h" tone="green" />
        <VitalsCell value={vitals?.deadEnded ?? 'unknown'} label="dead-ended · need you" tone="red" />
        <VitalsCell value={vitals?.throughput ?? 'unknown'} label="throughput" />
        <VitalsCell value={vitals?.medianReviewTime ?? 'unknown'} label="median time-in-review" />
        <div className="bg-raise p-[16px_22px] flex flex-col justify-center">
          <span className="font-mono text-[11.5px] text-dim">{timeWindow} window</span>
          <span className="font-mono text-[11px] text-ghost mt-1.5 select-none">
            {vitals?.windowStart && vitals?.windowEnd
              ? `${vitals.windowStart} → ${vitals.windowEnd}`
              : 'unknown'}
          </span>
        </div>
      </div>

      {/* Canvas */}
      <div>
        {view === 'pipeline' ? (
          /* Pipeline View */
          <div className="relative flex items-stretch border-t border-b border-line max-[600px]:flex-col overflow-x-auto max-[980px]:scrollbar-thin min-w-0">
            {/* Conduit Pipe Line */}
            <div className="absolute left-0 right-0 top-[64px] h-[1px] bg-line max-[600px]:hidden z-0" />

            {/* Intake cap (no bg-bg and z-10 for transparent conduit line) */}
            <div className="flex-[0_0_88px] max-[980px]:flex-[0_0_110px] max-[600px]:flex-[1_1_auto] p-[20px_14px] relative cursor-not-allowed hover:bg-[color-mix(in_oklab,var(--raise)_45%,var(--bg))] transition-colors">
              <span className="absolute top-[59px] left-4 w-2.5 h-2.5 rounded-full border-2 border-bg bg-line-2 z-20 max-[600px]:hidden" />
              <h2 className="font-display font-semibold text-[13px] tracking-[0.01em] text-ghost">Intake</h2>
              <div className="font-mono text-[10.5px] text-ghost mt-[34px] leading-relaxed">
                {intakeDetails.map((line, idx) => (
                  <span key={idx}>
                    {line}
                    {idx < intakeDetails.length - 1 && <br />}
                  </span>
                ))}
              </div>
            </div>

            {/* Design Stage */}
            <div className="flex-1 max-[980px]:flex-[0_0_250px] max-[600px]:flex-[1_1_auto] min-w-0 min-h-[300px] max-[600px]:min-h-0 p-[20px_20px] relative border-l border-line max-[600px]:border-l-0 max-[600px]:border-t cursor-not-allowed bg-bg hover:bg-[color-mix(in_oklab,var(--raise)_45%,var(--bg))] transition-colors z-10">
              <span className="absolute top-[59px] left-[22px] w-2.5 h-2.5 rounded-full bg-line-2 border-2 border-bg z-20 max-[600px]:hidden" />
              <div className="flex items-start justify-between">
                <h2 className="font-display font-semibold text-[14px] tracking-[0.01em] text-faint">Design</h2>
                <span className={cn(
                  "font-display font-bold text-[32px] leading-[0.8] tracking-[-0.02em]",
                  showData ? "text-fg" : "text-ghost"
                )}>
                  {showData ? designGoals.length : '—'}
                </span>
              </div>
              <div className="font-mono text-[11px] text-ghost mt-[30px]">
                <b className="text-faint font-medium">{designIo.inCount}</b> in · <b className="text-faint font-medium">{designIo.outCount}</b> out · {timeWindow}
              </div>
              {showData && designGoals.length > 0 && (
                <div className="mt-3.5 flex flex-col gap-2">
                  {designGoals.map((g) => (
                    <div key={g.id} className="flex items-start gap-2 py-2 border-t border-[color-mix(in_oklab,var(--line)_55%,transparent)] first-of-type:border-t-0 text-[12.5px] text-dim min-w-0">
                      <span className="w-1.5 h-1.5 rounded-full bg-ghost flex-shrink-0 mt-1.5" />
                      <span className="font-mono text-[11.5px] text-faint flex-shrink-0">#{g.id}</span>
                      <span className="min-w-0 flex-1 leading-[1.34] text-fg line-clamp-2 min-h-[2.68em]">{g.title}</span>
                      <span className="font-mono text-[11px] text-ghost flex-shrink-0 pl-1.5">{g.age}</span>
                    </div>
                  ))}
                  {stageMore?.['Design'] && (
                    <div className="font-mono text-[11px] text-ghost pt-2.5">{stageMore['Design']}</div>
                  )}
                </div>
              )}
            </div>

            {/* Build Stage */}
            <div className="flex-1 max-[980px]:flex-[0_0_250px] max-[600px]:flex-[1_1_auto] min-w-0 min-h-[300px] max-[600px]:min-h-0 p-[20px_20px] relative border-l border-line max-[600px]:border-l-0 max-[600px]:border-t cursor-not-allowed bg-bg hover:bg-[color-mix(in_oklab,var(--raise)_45%,var(--bg))] transition-colors z-10">
              <span className="absolute top-[59px] left-[22px] w-2.5 h-2.5 rounded-full bg-line-2 border-2 border-bg z-20 max-[600px]:hidden" />
              <div className="flex items-start justify-between">
                <h2 className="font-display font-semibold text-[14px] tracking-[0.01em] text-faint">Build</h2>
                <span className={cn(
                  "font-display font-bold text-[32px] leading-[0.8] tracking-[-0.02em]",
                  showData ? "text-fg" : "text-ghost"
                )}>
                  {showData ? buildGoals.length : '—'}
                </span>
              </div>
              <div className="font-mono text-[11px] text-ghost mt-[30px]">
                <b className="text-faint font-medium">{buildIo.inCount}</b> in · <b className="text-faint font-medium">{buildIo.outCount}</b> out · {timeWindow}
              </div>
              {showData && buildGoals.length > 0 && (
                <div className="mt-3.5 flex flex-col gap-2">
                  {buildGoals.map((g) => (
                    <div key={g.id} className="flex items-start gap-2 py-2 border-t border-[color-mix(in_oklab,var(--line)_55%,transparent)] first-of-type:border-t-0 text-[12.5px] text-dim min-w-0">
                      <span className="w-1.5 h-1.5 rounded-full bg-ghost flex-shrink-0 mt-1.5" />
                      <span className="font-mono text-[11.5px] text-faint flex-shrink-0">#{g.id}</span>
                      <span className="min-w-0 flex-1 leading-[1.34] text-fg line-clamp-2 min-h-[2.68em]">{g.title}</span>
                      <span className="font-mono text-[11px] text-ghost flex-shrink-0 pl-1.5">{g.age}</span>
                    </div>
                  ))}
                  {stageMore?.['Build'] && (
                    <div className="font-mono text-[11px] text-ghost pt-2.5">{stageMore['Build']}</div>
                  )}
                </div>
              )}
            </div>

            {/* Review Stage (inset rules via absolutely-positioned 2px before: bar) */}
            <div
              className={cn(
                "flex-1 max-[980px]:flex-[0_0_250px] max-[600px]:flex-[1_1_auto] min-w-0 min-h-[300px] max-[600px]:min-h-0 p-[20px_20px] relative border-l border-line max-[600px]:border-l-0 max-[600px]:border-t cursor-not-allowed bg-bg hover:bg-[color-mix(in_oklab,var(--raise)_45%,var(--bg))] transition-colors z-10",
                showData && reviewGoals.some((g) => g.pressure) && "before:absolute before:inset-x-0 before:top-0 before:h-[2px] before:bg-gold"
              )}
            >
              <span className={cn(
                "absolute top-[59px] left-[22px] w-2.5 h-2.5 rounded-full border-2 border-bg z-20 max-[600px]:hidden",
                showData && reviewGoals.some((g) => g.pressure) ? "bg-gold" : "bg-line-2"
              )} />
              <div className="flex items-start justify-between">
                <h2 className={cn(
                  "font-display font-semibold text-[14px] tracking-[0.01em]",
                  showData && reviewGoals.some((g) => g.pressure) ? "text-gold" : "text-faint"
                )}>
                  Review
                </h2>
                <span className={cn(
                  "font-display font-bold text-[32px] leading-[0.8] tracking-[-0.02em]",
                  showData ? "text-fg" : "text-ghost"
                )}>
                  {showData ? reviewGoals.length : '—'}
                </span>
              </div>
              <div className="font-mono text-[11px] text-ghost mt-[30px]">
                <b className="text-faint font-medium">{reviewIo.inCount}</b> in · <b className="text-faint font-medium">{reviewIo.outCount}</b> out · {timeWindow}
              </div>
              {showData && reviewGoals.some((g) => g.pressure) && reviewPressureLabel && (
                <span className="inline-flex items-center gap-[7px] text-[11px] font-medium mt-[12px] px-2.5 py-1 rounded-[7px] border border-[color-mix(in_oklab,var(--gold)_38%,var(--line))] text-gold">
                  <span className="w-1.5 h-1.5 rounded-full bg-gold" />
                  {reviewPressureLabel}
                </span>
              )}
              {showData && reviewGoals.length > 0 && (
                <div className="mt-3.5 flex flex-col gap-2">
                  {reviewGoals.map((g) => (
                    <div key={g.id} className="flex items-start gap-2 py-2 border-t border-[color-mix(in_oklab,var(--line)_55%,transparent)] first-of-type:border-t-0 text-[12.5px] text-dim min-w-0">
                      <span className={cn("w-1.5 h-1.5 rounded-full flex-shrink-0 mt-1.5", g.pressure ? "bg-gold" : "bg-ghost")} />
                      <span className="font-mono text-[11.5px] text-faint flex-shrink-0">#{g.id}</span>
                      <span className="min-w-0 flex-1 leading-[1.34] text-fg line-clamp-2 min-h-[2.68em]">{g.title}</span>
                      <span className="font-mono text-[11px] text-ghost flex-shrink-0 pl-1.5">{g.age}</span>
                    </div>
                  ))}
                  {stageMore?.['Review'] && (
                    <div className="font-mono text-[11px] text-ghost pt-2.5">{stageMore['Review']}</div>
                  )}
                </div>
              )}
            </div>

            {/* Ship Stage (inset rules via absolutely-positioned 2px before: bar) */}
            <div
              className={cn(
                "flex-1 max-[980px]:flex-[0_0_250px] max-[600px]:flex-[1_1_auto] min-w-0 min-h-[300px] max-[600px]:min-h-0 p-[20px_20px] relative border-l border-line max-[600px]:border-l-0 max-[600px]:border-t cursor-not-allowed bg-bg hover:bg-[color-mix(in_oklab,var(--raise)_45%,var(--bg))] transition-colors z-10",
                showData && shipGoals.length > 0 && "before:absolute before:inset-x-0 before:top-0 before:h-[2px] before:bg-red"
              )}
            >
              <span className={cn(
                "absolute top-[59px] left-[22px] w-2.5 h-2.5 rounded-full border-2 border-bg z-20 max-[600px]:hidden",
                showData && shipGoals.length > 0 ? "bg-red" : "bg-line-2"
              )} />
              <div className="flex items-start justify-between">
                <h2 className={cn(
                  "font-display font-semibold text-[14px] tracking-[0.01em]",
                  showData && shipGoals.length > 0 ? "text-red" : "text-faint"
                )}>
                  Ship
                </h2>
                <span className={cn(
                  "font-display font-bold text-[32px] leading-[0.8] tracking-[-0.02em]",
                  showData ? "text-fg" : "text-ghost"
                )}>
                  {showData ? shipGoals.length : '—'}
                </span>
              </div>
              <div className="font-mono text-[11px] text-ghost mt-[30px]">
                <b className="text-faint font-medium">{shipIo.inCount}</b> in · <b className="text-faint font-medium">{shipIo.outCount}</b> out · {timeWindow}
              </div>
              {showData && shipTag && (
                <span className="inline-flex items-center gap-[7px] text-[11px] font-medium mt-[12px] px-2.5 py-1 rounded-[7px] border border-[color-mix(in_oklab,var(--red)_45%,var(--line))] text-red">
                  <span className="w-1.5 h-1.5 rounded-full bg-red" />
                  {shipTag}
                </span>
              )}
              {showData && shipGoals.length > 0 && (
                <div className="mt-3.5 flex flex-col gap-2">
                  {shipGoals.map((g) => (
                    <div key={g.id} className="flex items-start gap-2 py-2 border-t border-[color-mix(in_oklab,var(--line)_55%,transparent)] first-of-type:border-t-0 text-[12.5px] text-dim min-w-0">
                      <span className={cn("w-1.5 h-1.5 rounded-full flex-shrink-0 mt-1.5", g.state === 'merging' ? "bg-red" : "bg-green")} />
                      <span className="font-mono text-[11.5px] text-faint flex-shrink-0">#{g.id}</span>
                      <span className="min-w-0 flex-1 leading-[1.34] text-fg line-clamp-2 min-h-[2.68em]">{g.title}</span>
                      <span className="font-mono text-[11px] text-ghost flex-shrink-0 pl-1.5">{g.age}</span>
                    </div>
                  ))}
                  {stageMore?.['Ship'] && (
                    <div className="font-mono text-[11px] text-ghost pt-2.5">{stageMore['Ship']}</div>
                  )}
                </div>
              )}
            </div>

            {/* Merged Cap (no bg-bg and z-10 for transparent conduit line) */}
            <div className="flex-[0_0_88px] max-[980px]:flex-[0_0_110px] max-[600px]:flex-[1_1_auto] p-[20px_14px] relative border-l border-line max-[600px]:border-l-0 max-[600px]:border-t cursor-not-allowed hover:bg-[color-mix(in_oklab,var(--raise)_45%,var(--bg))] transition-colors">
              <span className="absolute top-[59px] left-4 w-2.5 h-2.5 rounded-full border-2 border-bg bg-green z-20 max-[600px]:hidden" />
              <h2 className="font-display font-semibold text-[13px] tracking-[0.01em] text-ghost">Merged</h2>
              <div className="font-mono text-[10.5px] text-ghost mt-[34px] leading-relaxed">
                {mergedDetails.map((line, idx) => (
                  <span key={idx}>
                    {idx === 0 ? (
                      <b className="text-green font-semibold text-[12.5px]">
                        {showData ? mergedGoals.length : '—'}
                      </b>
                    ) : (
                      line
                    )}
                    {idx < mergedDetails.length - 1 && <br />}
                  </span>
                ))}
              </div>
            </div>

            {/* Empty State Overlay */}
            {!showData && (
              <div className="absolute inset-x-0 bottom-8 flex items-center justify-center text-ghost font-mono text-[12px] pointer-events-none max-[600px]:static max-[600px]:py-8 max-[600px]:border-t max-[600px]:border-line z-30">
                no GitHub plane connected — sign-in pending
              </div>
            )}
          </div>
        ) : (
          /* Board View (outer border boxes removed per reviews §9) */
          <div className="relative min-h-[300px]">
            <div className="board grid grid-cols-5 max-[1080px]:flex max-[1080px]:overflow-x-auto max-[1080px]:scrollbar-thin gap-3.5">
              {/* Columns */}
              {['Design', 'Build', 'Review', 'Ship', 'Merged'].map((stageName) => {
                const stageGoals = getStageGoals(stageName);
                const isReview = stageName === 'Review';
                const isShip = stageName === 'Ship';
                const isMerged = stageName === 'Merged';

                return (
                  <div key={stageName} className="bcol flex-1 max-[1080px]:flex-[0_0_240px] flex flex-col min-w-0">
                    <div
                      className={cn(
                        "bcol-hd flex items-center justify-between pb-[11px] mb-3 border-b border-line",
                        isReview && "border-[color-mix(in_oklab,var(--gold)_42%,var(--line))]",
                        isShip && "border-[color-mix(in_oklab,var(--red)_42%,var(--line))]"
                      )}
                    >
                      <h2
                        className={cn(
                          "bnm font-display font-semibold text-[13px] tracking-[0.01em]",
                          isReview && "text-gold",
                          isShip && "text-red",
                          isMerged && "text-green",
                          !isReview && !isShip && !isMerged && "text-faint"
                        )}
                      >
                        {stageName}
                      </h2>
                      <span
                        className={cn(
                          "bct font-display font-bold text-[15px]",
                          isMerged ? "text-green" : showData ? "text-fg" : "text-ghost"
                        )}
                      >
                        {showData ? stageGoals.length : '—'}
                      </span>
                    </div>

                    <div className="bcards flex flex-col gap-2.5 min-h-[150px]">
                      {showData && stageGoals.map((g) => (
                        <div
                          key={g.id}
                          className="card bg-raise border border-line rounded-card p-3 cursor-not-allowed hover:bg-raise-2 hover:border-line-2 transition-colors flex flex-col gap-2"
                        >
                          <div className="flex items-center justify-between gap-2">
                            <span className="font-mono text-[11px] text-faint">#{g.id}</span>
                            <span
                              className={cn(
                                "font-mono text-[10px] px-1.5 py-0.5 rounded-[5px] border border-line-2 text-ghost lowercase",
                                g.state === 'merging' && g.pressure && "text-red border-[color-mix(in_oklab,var(--red)_45%,var(--line))]",
                                g.state === 'merged' && "text-green border-[color-mix(in_oklab,var(--green)_40%,var(--line))]"
                              )}
                            >
                              {g.gated ? `${g.state} · gated` : g.state}
                            </span>
                          </div>
                          <div className="text-[12.5px] text-fg leading-[1.34] line-clamp-2 min-h-[2.68em] font-ui">
                            {g.title}
                          </div>
                          <div className="font-mono text-[10.5px] text-ghost">
                            {g.repo || '—'} · {g.pr || '—'}
                          </div>
                        </div>
                      ))}
                      {showData && stageMore?.[stageName] && (
                        <div className="font-mono text-[11px] text-ghost pt-2">{stageMore[stageName]}</div>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>

            {/* Empty State Overlay */}
            {!showData && (
              <div className="absolute inset-0 flex items-center justify-center text-ghost font-mono text-[12px] pointer-events-none py-16 z-30">
                no GitHub plane connected — sign-in pending
              </div>
            )}
          </div>
        )}
      </div>

      {/* Needs you band */}
      <div className="mt-2.5">
        <div className="flex items-baseline gap-3 mb-3">
          <h2 className="font-mono text-eyebrow text-ghost uppercase">Needs you</h2>
          <span className={cn("font-mono text-[11.5px] select-none", needsYou && needsYou.length > 0 ? "text-red" : "text-ghost")}>
            {needsYou && needsYou.length > 0
              ? `${needsYou.length} · Attention · terminal outcomes & real writes (from GitHub)`
              : '— · terminal outcomes & real writes'}
          </span>
        </div>

        {needsYou === undefined ? (
          <div className="border border-dashed border-line rounded-panel p-6 bg-raise/50 flex flex-col items-center justify-center text-center">
            <span className="text-faint font-mono text-[12px]">
              Needs-you unavailable — requires GitHub plane (NyxID) integration
            </span>
          </div>
        ) : needsYou.length === 0 ? (
          <div className="border border-dashed border-line rounded-panel p-6 bg-raise/50 flex flex-col items-center justify-center text-center">
            <span className="text-faint font-mono text-[12px]">
              Nothing needs you
            </span>
          </div>
        ) : (
          <div className="flex flex-col border border-line rounded-panel overflow-hidden bg-line gap-px">
            {needsYou.map((item, idx) => (
              <div
                key={idx}
                className="flex items-center gap-4 p-3 bg-raise hover:bg-[color-mix(in_oklab,var(--raise)_30%,transparent)] transition-colors max-[780px]:flex-wrap max-[780px]:gap-2"
              >
                <span
                  className={cn(
                    "text-[10.5px] font-semibold tracking-[0.07em] uppercase w-[104px] flex-shrink-0",
                    item.leadTone === 'red' && "text-red",
                    item.leadTone === 'gold' && "text-gold",
                    !item.leadTone && "text-faint"
                  )}
                >
                  {item.lead}
                </span>
                <div className="flex-1 min-w-0 max-[780px]:w-full max-[780px]:flex-none">
                  <div className="font-medium text-[13px] text-fg">
                    {item.title}
                    {item.id && <span className="font-mono text-[11px] text-ghost ml-1.5">#{item.id}</span>}
                    {item.pr && <span className="font-mono text-[11px] text-ghost ml-1.5">· PR {item.pr}</span>}
                  </div>
                  <div className="font-mono text-[11px] text-ghost mt-0.5">{item.why}</div>
                </div>
                <button
                  disabled
                  className={cn(
                    "font-ui font-medium text-[12px] border border-line-2 bg-raise text-dim rounded-control px-3 py-1.5 cursor-not-allowed opacity-50 select-none",
                    item.actionTone === 'red' && "border-[color-mix(in_oklab,var(--red)_50%,var(--line))] text-red"
                  )}
                >
                  {item.actionLabel}
                </button>
                <button disabled className="font-ui font-medium text-[12px] border border-line-2 bg-raise text-dim rounded-control px-3 py-1.5 cursor-not-allowed opacity-50 select-none">
                  Open
                </button>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Footer */}
      <div className="foot flex gap-6 font-mono text-[11px] text-ghost flex-wrap mt-6 pt-[14px] border-t border-line">
        <span>counts &amp; rates scoped to <b>{timeWindow}</b></span>
        <span>merges → <b>integration branch</b> · a rollup PR carries integration → dev</span>
        <span>state re-derived from GitHub each poll · <b>labels are hints, markers are fact</b></span>
        <span>state as of <b>unknown</b> · poll-derived</span>
      </div>
    </div>
  );
}
