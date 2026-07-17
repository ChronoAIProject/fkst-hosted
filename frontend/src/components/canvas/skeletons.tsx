import { useContent } from '@/i18n';

// Loading skeletons for the canvas and the sidebar: shimmer blocks in the
// shape of the content they precede, so the first paint never blanks or
// snaps. Both are polite live regions announcing the loading state.

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
        <div key={k} className="anim-shimmer rounded-card h-[130px]" />
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
      <div className="anim-shimmer rounded-card h-[120px] mt-2" />
      <div className="anim-shimmer rounded-card h-[120px]" />
    </div>
  );
}
