import React from 'react';
import { cn } from '@/lib/utils';

export interface StateDotProps {
  tone: 'green' | 'red' | 'gold' | 'neutral';
  label: string;
  className?: string;
}

/**
 * StateDot component
 * 
 * Renders a small color dot alongside a text label.
 * 
 * CRITICAL LAW (Colorblind-safety): The label must NAME the state.
 * Dot shape/color alone must never carry meaning, and two different-tone
 * dots with the same label would be hue-only, which is forbidden.
 */
export const StateDot: React.FC<StateDotProps> = ({ tone, label, className }) => {
  const toneClasses = {
    green: 'bg-green',
    red: 'bg-red',
    gold: 'bg-gold',
    neutral: 'bg-faint',
  };

  return (
    <div className={cn('inline-flex items-center gap-2 font-ui text-body text-fg', className)}>
      <span
        className={cn(
          'h-2 w-2 rounded-full flex-shrink-0',
          toneClasses[tone]
        )}
        aria-hidden="true"
      />
      <span>{label}</span>
    </div>
  );
};
