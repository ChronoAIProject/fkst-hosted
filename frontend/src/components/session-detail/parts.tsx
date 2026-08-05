import type { CSSProperties, ReactNode } from 'react';
import { cn } from '@/lib/utils';

/**
 * The two-pane split the session-detail tabs are built on: two panes side by
 * side from `md`, stacked below it.
 *
 * One grid, so the panes are always equal height; `flex-1 min-h-0` so the height
 * comes from the tab panel and each pane's overflow is its own job rather than
 * escaping upward — which would scroll one pane out of view while reading the
 * other. A single column below `md`, where two panes have no room.
 *
 * `startTrack` is the FIRST column's grid track, so the same machinery serves
 * both shapes this surface needs: a fixed-width navigation rail beside a detail
 * pane (`'11.5rem'`, the default), or two peer panes (`'minmax(0,1fr)'`). It
 * rides a CSS variable because Tailwind's JIT cannot generate a class from a
 * runtime string: `md:grid-cols-[${w}]` silently produces no CSS, whereas the
 * literal `var(--split-start)` form below is scanned from this source.
 *
 * Both panes must carry `min-h-0` themselves. A grid item's `min-height`
 * defaults to `auto`, i.e. its content height, so the row would grow past the
 * container and the panes' own scrollers would never engage — the tab would
 * scroll instead, dragging one pane out of view. Passing a pane wrapped in an
 * extra `<div>` breaks this the same way: the wrapper becomes the grid item.
 */
export function SplitPanes({
  start,
  end,
  startTrack = '11.5rem',
}: {
  start: ReactNode;
  end: ReactNode;
  startTrack?: string;
}) {
  return (
    <div
      style={{ '--split-start': startTrack } as CSSProperties}
      className="grid gap-4 md:grid-cols-[var(--split-start)_minmax(0,1fr)] flex-1 min-h-0"
    >
      {start}
      {end}
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
