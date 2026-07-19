import React from 'react';
import { cn } from '@/lib/utils';

/** Tiny mono badge used across the dashboard for states and labels. */
export function Chip({
  children,
  tone = 'neutral',
}: {
  children: React.ReactNode;
  tone?: 'neutral' | 'amber' | 'green' | 'red';
}) {
  return (
    <span
      className={cn(
        // Elevated chip: a subtly status-tinted surface + a soft, status-matched
        // glow so a state reads at a glance — the glow is decorative reinforcement
        // only; the text label always carries the meaning (never color/glow alone).
        'font-mono text-[10.5px] px-1.5 py-0.5 rounded-chip border',
        tone === 'amber' &&
          'text-amber bg-[color-mix(in_oklab,var(--amber)_12%,var(--raise-2))] border-[color-mix(in_oklab,var(--amber)_40%,var(--line))] shadow-glow-amber',
        tone === 'green' &&
          'text-green bg-[color-mix(in_oklab,var(--green)_12%,var(--raise-2))] border-[color-mix(in_oklab,var(--green)_40%,var(--line))] shadow-glow-green',
        tone === 'red' &&
          'text-red bg-[color-mix(in_oklab,var(--red)_12%,var(--raise-2))] border-[color-mix(in_oklab,var(--red)_40%,var(--line))] shadow-glow-red',
        // Neutral stays quiet: raised surface, hairline, no glow.
        tone === 'neutral' && 'text-ghost bg-raise-2 border-line-2'
      )}
    >
      {children}
    </span>
  );
}
