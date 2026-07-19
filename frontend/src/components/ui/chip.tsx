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
        'font-mono text-[10.5px] px-1.5 py-0.5 rounded-chip border',
        tone === 'amber' && 'text-amber border-[color-mix(in_oklab,var(--amber)_40%,var(--line))]',
        tone === 'green' && 'text-green border-[color-mix(in_oklab,var(--green)_40%,var(--line))]',
        tone === 'red' && 'text-red border-[color-mix(in_oklab,var(--red)_40%,var(--line))]',
        tone === 'neutral' && 'text-ghost border-line-2'
      )}
    >
      {children}
    </span>
  );
}
