import React from 'react';
import { cn } from '@/lib/utils';

export type CiStatus = 'passing' | 'unknown' | 'failing';

export interface CiGlyphProps {
  status: CiStatus;
  className?: string;
}

export const CiGlyph: React.FC<CiGlyphProps> = ({ status, className }) => {
  const glyphs = {
    passing: '✓',
    unknown: '—',
    failing: '✗',
  };

  const ariaLabels = {
    passing: 'CI passing',
    unknown: 'CI unknown',
    failing: 'CI failing',
  };

  const colorClasses = {
    passing: 'text-green',
    unknown: 'text-ghost',
    failing: 'text-red',
  };

  return (
    <span
      role="img"
      className={cn('font-mono text-[13px] font-medium select-none', colorClasses[status], className)}
      aria-label={ariaLabels[status]}
      title={ariaLabels[status]}
    >
      {glyphs[status]}
    </span>
  );
};
