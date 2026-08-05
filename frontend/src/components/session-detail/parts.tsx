import type { CSSProperties, ReactNode } from 'react';
import { cn } from '@/lib/utils';

/**
 * The master/detail split the session-detail tabs are built on: pick one item in
 * the left rail, read it on the right.
 *
 * One grid, so the two panes are always equal height; `flex-1 min-h-0` so the
 * height comes from the tab panel and each pane's overflow is its own job rather
 * than escaping upward — which would scroll the rail out of view while reading.
 * A single column below `md`, where a rail beside content has no room.
 *
 * The rail width rides a CSS variable because Tailwind's JIT cannot generate a
 * class from a runtime string: `md:grid-cols-[${w}]` silently produces no CSS,
 * whereas the literal `var(--rail-w)` form below is scanned from this source.
 */
export function MasterDetailSplit({
  rail,
  detail,
  railWidth = '11.5rem',
}: {
  rail: ReactNode;
  detail: ReactNode;
  railWidth?: string;
}) {
  return (
    <div
      style={{ '--rail-w': railWidth } as CSSProperties}
      className="grid gap-4 md:grid-cols-[var(--rail-w)_minmax(0,1fr)] flex-1 min-h-0"
    >
      {rail}
      {detail}
    </div>
  );
}

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
