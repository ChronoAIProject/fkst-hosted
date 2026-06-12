import React from 'react';
import { cn } from '@/lib/utils';
import { SessionStatus } from '@/lib/api/types';

export interface SessionStateBadgeProps {
  status: SessionStatus | undefined;
  className?: string;
}

export const SessionStateBadge: React.FC<SessionStateBadgeProps> = ({ status, className }) => {
  const getBadgeToneAndText = (state: SessionStatus | undefined) => {
    if (!state) return { tone: 'neutral' as const, text: 'unknown' };
    switch (state) {
      case 'running':
        return { tone: 'green' as const, text: 'running' };
      case 'stopped':
        return { tone: 'neutral' as const, text: 'stopped' };
      case 'failed':
        return { tone: 'red' as const, text: 'failed' };
      case 'stopping':
        return { tone: 'gold' as const, text: 'stopping' };
      case 'pending':
        return { tone: 'neutral' as const, text: 'pending' };
      case 'validating':
        return { tone: 'neutral' as const, text: 'validating' };
      default:
        return { tone: 'neutral' as const, text: state };
    }
  };

  const { tone, text } = getBadgeToneAndText(status);

  const badgeColors: Record<'green' | 'red' | 'gold' | 'neutral', string> = {
    green: 'border-[color-mix(in_oklab,var(--green)_40%,var(--line))] text-green',
    red: 'border-[color-mix(in_oklab,var(--red)_45%,var(--line))] text-red',
    gold: 'border-[color-mix(in_oklab,var(--gold)_40%,var(--line))] text-gold',
    neutral: 'border-line-2 text-dim',
  };

  return (
    <span
      className={cn(
        'inline-block font-mono text-[10.5px] font-medium tracking-[0.02em] px-2 py-[3px] rounded-chip border lowercase whitespace-nowrap bg-raise-2',
        badgeColors[tone],
        className
      )}
    >
      {text}
    </span>
  );
};
