import React from 'react';
import { cn } from '@/lib/utils';

export type StateBadgeState =
  | 'thinking'
  | 'ready'
  | 'implementing'
  | 'pr-open'
  | 'reviewing'
  | 'merge-ready'
  | 'merging'
  | 'fixing'
  | 'review-meta'
  | 'impl-failed'
  | 'blocked'
  | 'merged';

export type StateBadgeProps = {
  className?: string;
} & (
  | { state: 'ready'; gated?: boolean; pressure?: never }
  | { state: 'reviewing' | 'review-meta'; pressure?: boolean; gated?: never }
  | { state: Exclude<StateBadgeState, 'ready' | 'reviewing' | 'review-meta'>; gated?: never; pressure?: never }
);

export const StateBadge: React.FC<StateBadgeProps> = ({
  state,
  pressure = false,
  gated = false,
  className,
}) => {
  // Constrain gated to the 'ready' substate only per the mockup (ready · gated)
  const isGated = state === 'ready' && gated;
  const isPressured = (state === 'reviewing' || state === 'review-meta') && pressure;

  // Determine tone based on the 12-state rules
  let tone: 'green' | 'red' | 'gold' | 'neutral' = 'neutral';
  if (state === 'merged') {
    tone = 'green';
  } else if (state === 'blocked' || state === 'impl-failed' || state === 'merging') {
    tone = 'red';
  } else if (isPressured) {
    tone = 'gold';
  }

  // Format label text in lowercase per the goals.html mockup badge metrics
  const labelText = isGated ? `${state} · gated` : state;

  // Custom inline styles for color-mix aligning with the goals.html mockup exactly
  const inlineStyles: React.CSSProperties = {};
  if (!isGated && tone !== 'neutral') {
    const toneVar = `var(--${tone})`;
    inlineStyles.color = toneVar;
    
    // Tinted border mix percentages verbatim from goals.html mockup
    const percent = tone === 'red' ? '45%' : '40%';
    inlineStyles.borderColor = `color-mix(in oklab, ${toneVar} ${percent}, var(--line))`;
  }

  // TODO(design): non-hue pressure affix
  // For colorblind safety, add descriptive title and aria-label when under pressure
  const accessibilityAttrs = isPressured
    ? {
        title: `${state} — under pressure`,
        'aria-label': `${state} — under pressure`,
      }
    : {
        title: labelText,
        'aria-label': labelText,
      };

  return (
    <span
      className={cn(
        'inline-block font-mono text-[10.5px] font-medium tracking-[0.02em] px-2 py-[3px] rounded-chip border lowercase whitespace-nowrap bg-raise',
        isGated
          ? 'border-dashed border-line-2 text-faint'
          : tone === 'neutral'
          ? 'border-line-2 text-dim'
          : '',
        className
      )}
      style={inlineStyles}
      {...accessibilityAttrs}
    >
      {labelText}
    </span>
  );
};
