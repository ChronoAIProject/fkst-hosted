import React from 'react';
import { cn } from '@/lib/utils';

export type FreshnessState = 'fresh' | 'syncing' | 'stale-warn' | 'stale-critical' | 'unknown';

export interface FreshnessChipProps {
  source: string;
  asOf?: string;
  state: FreshnessState;
  className?: string;
}

/**
 * FreshnessChip component
 * 
 * Displays the synchronization state and freshness of a data source.
 * 
 * Precomputed-state contract:
 * - The caller computes the state from poll thresholds:
 *   - 'fresh' / 'syncing': normal operation, neutral
 *   - 'stale-warn': exactly 1 missed poll (gold tone)
 *   - 'stale-critical': 2 or more missed polls (red tone)
 *   - 'unknown': process environment / hosted service is unreachable (ghost tone)
 * - `asOf` is preformatted display text (e.g. "30s ago", "5m ago").
 */
export const FreshnessChip: React.FC<FreshnessChipProps> = ({
  source,
  asOf,
  state,
  className,
}) => {
  // Emit distinct wording per tier to avoid hue-only semantic indicators
  const getDisplayText = () => {
    switch (state) {
      case 'syncing':
        return `${source} · syncing…`;
      case 'unknown':
        return `${source} · unknown`;
      case 'stale-warn':
        return `${source} · stale · ${asOf || '—'}`;
      case 'stale-critical':
        return `${source} · stale 2+ polls · ${asOf || '—'}`;
      default:
        return `${source} · ${asOf || '—'}`;
    }
  };

  const isStale = state === 'stale-warn' || state === 'stale-critical';
  const tone = state === 'stale-warn' ? 'gold' : 'red';

  const inlineStyles: React.CSSProperties = {};
  if (isStale) {
    const toneVar = `var(--${tone})`;
    inlineStyles.color = toneVar;
    inlineStyles.borderColor = `color-mix(in oklab, ${toneVar} 38%, var(--line))`;
    inlineStyles.backgroundColor = `color-mix(in oklab, ${toneVar} 12%, transparent)`;
  }

  const baseClasses = {
    fresh: 'text-faint bg-raise border-line',
    syncing: 'text-faint bg-raise border-line',
    'stale-warn': '',
    'stale-critical': '',
    unknown: 'text-ghost bg-raise border-line',
  };

  return (
    <div
      className={cn(
        'inline-flex items-center gap-[7px] font-mono text-[11.5px] tracking-[0.02em] px-[10px] py-[5px] rounded-[8px] border whitespace-nowrap',
        baseClasses[state],
        className
      )}
      style={inlineStyles}
    >
      <span>{getDisplayText()}</span>
    </div>
  );
};
