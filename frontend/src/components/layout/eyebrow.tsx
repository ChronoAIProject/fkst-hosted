import React from 'react';
import { cn } from '@/lib/utils';

export interface EyebrowProps extends React.HTMLAttributes<HTMLDivElement> {
  children: React.ReactNode;
}

export function Eyebrow({ children, className, ...props }: EyebrowProps) {
  return (
    <div
      className={cn(
        // Brighter label (ghost -> faint) + a small amber->gold gradient tick in
        // place of the flat dash, giving the eyebrow a touch of brand energy.
        'text-eyebrow font-semibold font-mono uppercase text-faint flex items-center min-w-0',
        className
      )}
      {...props}
    >
      <span className="w-[18px] h-px bg-grad-accent mr-2 flex-none" aria-hidden="true" />
      <span className="truncate min-w-0">{children}</span>
    </div>
  );
}
