import React from 'react';
import { cn } from '@/lib/utils';

// W2.F1 note: borderless composition inside a parent panel is intended via `border-0 rounded-none`
export interface TriPanelProps extends React.HTMLAttributes<HTMLDivElement> {
  children: React.ReactNode;
}

export function TriPanel({ children, className, ...props }: TriPanelProps) {
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

export interface TriPanelCellProps extends Omit<React.HTMLAttributes<HTMLDivElement>, 'title'> {
  dotClassName?: string;
  header: React.ReactNode;
  title: React.ReactNode;
  body: React.ReactNode;
  tagSlot?: React.ReactNode;
}

export function TriPanelCell({
  dotClassName = 'bg-ghost',
  header,
  title,
  body,
  tagSlot,
  className,
  ...props
}: TriPanelCellProps) {
  return (
    <div
      className={cn(
        'bg-raise p-6 flex flex-col gap-3 min-w-0',
        className
      )}
      {...props}
    >
      <div className="flex items-center gap-2 min-w-0">
        <span className={cn('w-2 h-2 rounded-sm flex-none', dotClassName)} />
        <span className="text-[10px] tracking-[0.1em] font-mono uppercase text-ghost truncate min-w-0">
          {header}
        </span>
      </div>
      <h3 className="font-display font-semibold text-[16px] tracking-[0.01em] text-fg truncate min-w-0 leading-tight">
        {title}
      </h3>
      <p className="text-body text-dim min-w-0 flex-1 leading-relaxed">
        {body}
      </p>
      {tagSlot && (
        <div className="flex-none flex items-center min-w-0 mt-2">
          {tagSlot}
        </div>
      )}
    </div>
  );
}
