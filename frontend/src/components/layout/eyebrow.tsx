import React from 'react';
import { cn } from '@/lib/utils';

export interface EyebrowProps extends React.HTMLAttributes<HTMLDivElement> {
  children: React.ReactNode;
}

export function Eyebrow({ children, className, ...props }: EyebrowProps) {
  return (
    <div
      className={cn(
        'text-eyebrow font-semibold font-mono uppercase text-ghost flex items-center min-w-0',
        className
      )}
      {...props}
    >
      <span className="w-[18px] h-px bg-ghost mr-2 flex-none" aria-hidden="true" />
      <span className="truncate min-w-0">{children}</span>
    </div>
  );
}
