import React from 'react';
import { cn } from '@/lib/utils';

// Margin convention: consumers should apply mt-[34px] mb-[14px]
export interface SectionHeadingProps extends React.HTMLAttributes<HTMLDivElement> {
  children: React.ReactNode;
  count?: React.ReactNode;
}

export function SectionHeading({
  children,
  count,
  className,
  ...props
}: SectionHeadingProps) {
  return (
    <div
      className={cn('flex items-baseline gap-3 flex-wrap min-w-0', className)}
      {...props}
    >
      <h2 className="font-display text-[16px] font-semibold tracking-[0.01em] text-fg truncate min-w-0">
        {children}
      </h2>
      {count !== undefined && (
        <span className="font-mono text-[11.5px] text-ghost tabular-nums min-w-0 select-none">
          {count}
        </span>
      )}
    </div>
  );
}
