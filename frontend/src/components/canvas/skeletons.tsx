import { useContent } from '@/i18n';

// Loading skeletons for the canvas and the sidebar: shimmer blocks in the
// shape of the content they precede, so the first paint never blanks or
// snaps. Both are polite live regions announcing the loading state.
//
// Elevated look: card-sized placeholders wear the same hairline border +
// layered depth + inner top highlight as the real cards they precede, so the
// loading state reads as glass panels filling in rather than flat grey boxes.
// The animated gradient sweep (.anim-shimmer) stays the moving highlight; it
// collapses to a static fill under prefers-reduced-motion.
const cardChrome = 'border border-line shadow-[var(--shadow-1),var(--highlight-top)]';

const cardKeys = ['a', 'b', 'c', 'd', 'e', 'f'] as const;

export function CanvasSkeleton() {
  const cc = useContent().dashboard.canvas;
  return (
    <div
      role="status"
      aria-label={cc.loadingCanvas}
      data-testid="canvas-skeleton"
      className="grid grid-cols-3 max-[900px]:grid-cols-2 gap-6 p-6 h-full content-start"
    >
      {cardKeys.map((k) => (
        <div key={k} className={`anim-shimmer rounded-card h-[130px] ${cardChrome}`} />
      ))}
    </div>
  );
}

const lineWidths = ['w-2/3', 'w-full', 'w-5/6', 'w-1/2', 'w-full', 'w-3/4'] as const;

export function SidebarSkeleton() {
  const cc = useContent().dashboard.canvas;
  return (
    <div
      role="status"
      aria-label={cc.loadingSidebar}
      data-testid="sidebar-skeleton"
      className="flex flex-col gap-3 p-1"
    >
      <div className="anim-shimmer rounded-chip h-4 w-1/3" />
      {lineWidths.map((w, i) => (
        <div key={`${w}-${i}`} className={`anim-shimmer rounded-chip h-3 ${w}`} />
      ))}
      <div className={`anim-shimmer rounded-card h-[120px] mt-2 ${cardChrome}`} />
      <div className={`anim-shimmer rounded-card h-[120px] ${cardChrome}`} />
    </div>
  );
}
