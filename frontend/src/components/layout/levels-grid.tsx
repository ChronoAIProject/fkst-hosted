import React from 'react';
import { cn } from '@/lib/utils';

// W2.F1 note: borderless composition inside a parent panel is intended via `border-0 rounded-none`
export interface LevelsGridProps extends React.HTMLAttributes<HTMLDivElement> {
  children: React.ReactNode;
}

export function LevelsGrid({ children, className, ...props }: LevelsGridProps) {
  return (
    <div
      className={cn(
        'rounded-panel overflow-hidden border border-line bg-line grid grid-cols-1 min-[981px]:grid-cols-3 gap-px',
        className
      )}
      {...props}
    >
      {children}
    </div>
  );
}

export interface LevelsGridCellProps extends React.HTMLAttributes<HTMLDivElement> {
  eyebrow: React.ReactNode; // kicker
  value: React.ReactNode;
  description: React.ReactNode;
}

export function LevelsGridCell({
  eyebrow,
  value,
  description,
  className,
  ...props
}: LevelsGridCellProps) {
  return (
    <div
      className={cn(
        'bg-raise py-4 px-[22px] flex flex-col gap-2 min-w-0',
        className
      )}
      {...props}
    >
      <div className="text-[10px] tracking-[0.13em] font-mono uppercase text-ghost truncate min-w-0">
        {eyebrow}
      </div>
      <div className="font-display font-semibold text-[16px] tracking-[0.01em] text-fg truncate min-w-0 leading-tight">
        {value}
      </div>
      <div className="font-mono text-[11px] text-faint leading-[1.55] min-w-0 mt-auto">
        {description}
      </div>
    </div>
  );
}
