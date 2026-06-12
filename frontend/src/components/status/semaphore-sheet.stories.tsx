import type { Meta } from '@storybook/react-vite';
import { StateDot } from './state-dot';
import { StateBadge, type StateBadgeState } from './state-badge';
import { CiGlyph } from './ci-glyph';
import { FreshnessChip } from './freshness-chip';
import { PostureChip } from './posture-chip';
import { VitalsCell } from './vitals-cell';

// Note: Loading/empty/error states are N/A because this is a purely static presentation component.

const meta: Meta = {
  title: 'Status/SemaphoreSheet',
  parameters: {
    layout: 'fullscreen',
  },
};

export default meta;

const ALL_STATES: StateBadgeState[] = [
  'thinking',
  'ready',
  'implementing',
  'pr-open',
  'reviewing',
  'merge-ready',
  'merging',
  'fixing',
  'review-meta',
  'impl-failed',
  'blocked',
  'merged',
];

export const VocabularySheet = () => {
  return (
    <div className="p-8 bg-bg text-fg min-h-screen font-ui">
      <div className="max-w-shell mx-auto flex flex-col gap-8">
        <div>
          <h1 className="font-display font-semibold text-[19px] tracking-tight mb-2">
            Status & Freshness Component Vocabulary
          </h1>
          <p className="text-body text-dim max-w-2xl">
            Wave 1 Lane C status components conforming to the color design specification.
            No color-only semantics, fully colorblind-safe, using oklch color-mix tint recipe.
          </p>
        </div>

        {/* 1. StateDot */}
        <section className="border border-line bg-raise p-6 rounded-panel flex flex-col gap-4">
          <h2 className="text-eyebrow text-ghost">1. StateDot (Label + Dot)</h2>
          <div className="flex flex-wrap gap-6">
            <StateDot tone="green" label="Healthy / Success" />
            <StateDot tone="red" label="Danger / Failure" />
            <StateDot tone="gold" label="Warning / Bottle-neck" />
            <StateDot tone="neutral" label="Neutral / In Flight" />
          </div>
        </section>

        {/* 2. StateBadge */}
        <section className="border border-line bg-raise p-6 rounded-panel flex flex-col gap-4">
          <h2 className="text-eyebrow text-ghost">2. StateBadge (12-State Chips)</h2>
          <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
            {ALL_STATES.map((state) => (
              <div key={state} className="flex flex-col gap-2 p-3 bg-bg border border-line rounded-card">
                <span className="text-ghost text-[10px] font-mono uppercase">{state}</span>
                <div className="flex flex-wrap items-center gap-2 mt-1">
                  <StateBadge state={state} />
                  {(state === 'reviewing' || state === 'review-meta') && (
                    <StateBadge state={state} pressure />
                  )}
                  {state === 'ready' && (
                    <StateBadge state={state} gated />
                  )}
                </div>
              </div>
            ))}
          </div>
        </section>

        {/* 3. CiGlyph & 5. PostureChip */}
        <div className="grid grid-cols-1 md:grid-cols-2 gap-8">
          {/* 3. CiGlyph */}
          <section className="border border-line bg-raise p-6 rounded-panel flex flex-col gap-4">
            <h2 className="text-eyebrow text-ghost">3. CiGlyph (CI Indicators)</h2>
            <div className="flex items-center gap-8 bg-bg p-4 border border-line rounded-card">
              <div className="flex items-center gap-2">
                <CiGlyph status="passing" />
                <span className="text-dim text-body">Passing</span>
              </div>
              <div className="flex items-center gap-2">
                <CiGlyph status="unknown" />
                <span className="text-dim text-body">Unknown</span>
              </div>
              <div className="flex items-center gap-2">
                <CiGlyph status="failing" />
                <span className="text-dim text-body">Failing</span>
              </div>
            </div>
          </section>

          {/* 5. PostureChip */}
          <section className="border border-line bg-raise p-6 rounded-panel flex flex-col gap-4">
            <h2 className="text-eyebrow text-ghost">5. PostureChip (v1 Law Fixed Posture)</h2>
            <div className="flex items-center bg-bg p-4 border border-line rounded-card">
              <PostureChip />
            </div>
          </section>
        </div>

        {/* 4. FreshnessChip */}
        <section className="border border-line bg-raise p-6 rounded-panel flex flex-col gap-4">
          <h2 className="text-eyebrow text-ghost">4. FreshnessChip (Sync Status)</h2>
          <div className="flex flex-wrap gap-4 bg-bg p-4 border border-line rounded-card">
            <div className="flex flex-col gap-1">
              <span className="text-[10px] text-ghost font-mono uppercase">Fresh</span>
              <FreshnessChip source="github" asOf="30s ago" state="fresh" />
            </div>
            <div className="flex flex-col gap-1">
              <span className="text-[10px] text-ghost font-mono uppercase">Syncing</span>
              <FreshnessChip source="github" state="syncing" />
            </div>
            <div className="flex flex-col gap-1">
              <span className="text-[10px] text-ghost font-mono uppercase">Stale (Warn)</span>
              <FreshnessChip source="github" asOf="5m ago" state="stale-warn" />
            </div>
            <div className="flex flex-col gap-1">
              <span className="text-[10px] text-ghost font-mono uppercase">Stale (Critical)</span>
              <FreshnessChip source="github" asOf="15m ago" state="stale-critical" />
            </div>
            <div className="flex flex-col gap-1">
              <span className="text-[10px] text-ghost font-mono uppercase">Unknown</span>
              <FreshnessChip source="github" state="unknown" />
            </div>
          </div>
        </section>

        {/* 6. VitalsCell */}
        <section className="border border-line bg-raise p-6 rounded-panel flex flex-col gap-4">
          <h2 className="text-eyebrow text-ghost">6. VitalsCell (Vitals)</h2>
          <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
            <VitalsCell value={13} label="in flight now" />
            <VitalsCell value={22} label="merged · 24h" tone="green" />
            <VitalsCell value={3} label="dead-ended · need you" tone="red" />
            <VitalsCell value="unknown" label="median time-in-review" />
          </div>
        </section>
      </div>
    </div>
  );
};
