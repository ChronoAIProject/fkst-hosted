import React from 'react';
import { cn } from '@/lib/utils';

export interface VitalsCellProps {
  value: number | string | 'unknown';
  label: string;
  tone?: 'green' | 'red';
  className?: string;
}

export const VitalsCell: React.FC<VitalsCellProps> = ({
  value,
  label,
  tone,
  className,
}) => {
  return (
    <div
      className={cn(
        'bg-raise p-[16px_22px] flex flex-col justify-center',
        className
      )}
    >
      {value === 'unknown' ? (
        <span className="font-ui text-[18px] tracking-normal leading-[1.5] text-faint h-[27px] flex items-center select-none">
          unknown
        </span>
      ) : (
        <span
          className={cn(
            'font-display font-semibold text-[27px] tracking-[-0.02em] leading-none tabular-nums',
            tone === 'green' && 'text-green',
            tone === 'red' && 'text-red',
            !tone && 'text-fg'
          )}
        >
          {value}
        </span>
      )}
      <span className="text-[12px] text-faint mt-1.5 font-ui">
        {label}
      </span>
    </div>
  );
};
