import type { ReactNode } from 'react';
import { useContent } from '@/i18n';
import { FadeSwap } from '@/components/ui/motion';
import { ScrollArea } from '@/components/ui/scroll-area';
import { levelKey } from '@/components/canvas/level';
import type { CanvasLevel } from '@/components/canvas/level';

/** The level-aware right sidebar shell.
 *
 *  Layout: the shell is a flex column that fills the `flex-1 min-h-0` row the
 *  dashboard gives it, and delegates all scrolling to an internal ScrollArea
 *  (`h-full min-h-0 overflow-y-auto`). This replaces the former magic
 *  `max-h-[720px]` cap — which fought whatever height the row actually had —
 *  and the `max-h-none` narrow escape hatch that pushed overflow onto the
 *  window. On the stacked (<=1100px) layout the row no longer bounds the
 *  column, so the shell carries its own sensible min-height floor instead of
 *  collapsing to nothing.
 *
 *  Motion: content crossfades when the level changes AND when a level's body
 *  swaps from skeleton to loaded. The `loaded` flag is folded into the swap key
 *  so that within-level skeleton→content transition animates too — both cases
 *  ride one reduced-motion-safe FadeSwap (instant final state under
 *  prefers-reduced-motion). */
export function SidebarPanel({
  level,
  children,
  loaded = true,
}: {
  level: CanvasLevel;
  children: ReactNode;
  /** Whether the body currently rendered is the loaded content (vs a
   *  skeleton). Encoded into the crossfade key so the skeleton→content swap
   *  within a single level animates. Defaults to `true` so callers that don't
   *  distinguish load state get the level-only crossfade unchanged. */
  loaded?: boolean;
}) {
  const cc = useContent().dashboard.canvas;

  return (
    <aside
      aria-label={cc.sidebarAria}
      // flex column so the ScrollArea's flex-1 min-h-0 can fill it; h-full
      // consumes the row height on desktop, while the narrow layout falls back
      // to a min-height floor (h-auto) since the stacked column no longer
      // bounds it. overflow-hidden clips the scroll body to the rounded shell.
      className="w-[400px] max-[1100px]:w-full flex-none flex flex-col min-h-0 h-full max-[1100px]:h-auto max-[1100px]:min-h-[22rem] border border-line rounded-panel bg-raise overflow-hidden"
    >
      <ScrollArea className="p-5">
        <FadeSwap k={`${levelKey(level)}:${loaded ? 'ready' : 'loading'}`}>{children}</FadeSwap>
      </ScrollArea>
    </aside>
  );
}
