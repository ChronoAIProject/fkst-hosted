import React from 'react';
import { cn } from '@/lib/utils';

export interface HairlineListProps extends React.HTMLAttributes<HTMLDivElement> {
  children: React.ReactNode;
}

export function HairlineList({ children, className, ...props }: HairlineListProps) {
  return (
    <div
      className={cn(
        'rounded-panel overflow-hidden border border-line bg-line grid gap-px',
        className
      )}
      {...props}
    >
      {children}
    </div>
  );
}

export interface HairlineRowProps extends React.HTMLAttributes<HTMLDivElement> {
  leftContent: React.ReactNode;
  rightContent?: React.ReactNode;
}

export function HairlineRow({
  leftContent,
  rightContent,
  className,
  ...props
}: HairlineRowProps) {
  return (
    <div
      className={cn(
        'bg-raise py-4 px-5 flex items-center justify-between gap-4 hover:bg-[color-mix(in_oklab,var(--raise-2)_45%,var(--raise))] transition-colors min-w-0',
        className
      )}
      {...props}
    >
      <div className="min-w-0 flex-1">
        {leftContent}
      </div>
      {rightContent && (
        <div className="flex-none flex items-center gap-2">
          {rightContent}
        </div>
      )}
    </div>
  );
}
