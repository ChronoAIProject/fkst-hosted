import { forwardRef } from 'react';
import type { CSSProperties, ReactNode } from 'react';
import { cn } from '@/lib/utils';

export type ScrollAreaProps = {
  children: ReactNode;
  /** Extra classes merged onto the scroll container. */
  className?: string;
  /** Cap the region's height; anything taller scrolls internally instead of
   *  growing the page. Any CSS length (e.g. 720 → '720px', '60vh'). Omit to
   *  let the flex parent bound it via `flex-1 min-h-0`. */
  maxHeight?: number | string;
  /** Scroll axis. Default 'y' (vertical only); 'both' also allows horizontal. */
  axis?: 'y' | 'both';
};

/** Themed thin scrollbar. Standard `scrollbar-*` props cover Firefox; the
 *  WebKit pseudo-elements (Chrome/Safari) are styled globally elsewhere, so
 *  here we only guarantee the thin, token-tinted track cross-browser. Inline
 *  (not a Tailwind class) because `scrollbar-color` takes two color values a
 *  utility can't express, and this component owns no global CSS file. */
const SCROLLBAR_STYLE: CSSProperties = {
  scrollbarWidth: 'thin',
  // thumb, then track. The thumb carries a whisper of the amber brand (15% mix
  // into --line-2) so the scrollbar feels part of the elevated palette rather
  // than a neutral gray; the track stays transparent to read on any surface.
  scrollbarColor: 'color-mix(in oklab, var(--line-2) 85%, var(--amber)) transparent',
};

/**
 * Bounded internal scroll region encapsulating the `flex-1 min-h-0
 * overflow-y-auto` pattern. The `min-h-0` is MANDATORY: without it a flex
 * child refuses to shrink below its content size, so the overflow never
 * engages and the whole page scrolls instead of this region.
 *
 * The ref is forwarded to the scrolling element itself so callers (e.g. the
 * shell's condensed-header logic) can read `scrollTop` / attach a scroll
 * listener directly.
 */
export const ScrollArea = forwardRef<HTMLDivElement, ScrollAreaProps>(function ScrollArea(
  { children, className, maxHeight, axis = 'y' },
  ref
) {
  const style: CSSProperties = { ...SCROLLBAR_STYLE };
  // Normalize a bare number to px; pass any string length through untouched.
  if (maxHeight !== undefined) {
    style.maxHeight = typeof maxHeight === 'number' ? `${maxHeight}px` : maxHeight;
  }

  return (
    <div
      ref={ref}
      style={style}
      className={cn(
        // flex-safe: fill the flex parent but stay shrinkable so overflow is
        // this region's job, never the page's.
        'flex-1 min-h-0 overflow-y-auto',
        axis === 'both' ? 'overflow-x-auto' : 'overflow-x-hidden',
        className
      )}
    >
      {children}
    </div>
  );
});
