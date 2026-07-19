import type { ReactNode } from 'react';
import { cn } from '@/lib/utils';

/** Small indeterminate spinner matching the dashboard's Refresh affordance. */
export function Spinner({ className }: { className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        'anim-spin inline-block w-3 h-3 border border-line-2 border-t-amber rounded-full flex-none',
        className
      )}
    />
  );
}

/** Uppercase eyebrow label used to head a section inside the drawer. */
export function SectionLabel({ children }: { children: ReactNode }) {
  return <span className="font-mono text-eyebrow text-ghost uppercase">{children}</span>;
}

/** A muted mono note line (loading / empty / hint states). */
export function Note({ children }: { children: ReactNode }) {
  return <p className="font-mono text-[11.5px] text-ghost">{children}</p>;
}

/** A non-blocking amber staleness/notice line. Frosted glass surface with an
 *  amber left rule + soft amber bloom so it reads as an advisory without
 *  shouting; rises in on mount (collapses to its final state under
 *  prefers-reduced-motion). */
export function NoticeLine({ children }: { children: ReactNode }) {
  return (
    <p className="anim-notice-in bg-glass backdrop-blur-glass border border-line border-l-2 border-l-amber rounded-card shadow-[var(--shadow-1),var(--glow-amber),var(--highlight-top)] px-3 py-2 font-mono text-[11.5px] text-dim">
      {children}
    </p>
  );
}
