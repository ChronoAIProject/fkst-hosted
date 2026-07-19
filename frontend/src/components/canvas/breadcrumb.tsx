import { useContent } from '@/i18n';
import { parentLevel } from './level';
import type { CanvasLevel } from './level';

// Ancestor crumb: quiet by default, brightens on hover with an underline-grow
// (a gradient accent hairline that scales in from the left — .hover-underline).
const crumbButton =
  'hover-underline font-mono text-[12px] text-dim hover:text-fg transition-colors cursor-pointer px-1.5 py-1 rounded-chip';

/** Breadcrumb + back affordance above the canvas. The current level renders
 *  as plain text (aria-current); ancestors are buttons that jump straight to
 *  their level. Escape is handled page-side and mirrors the Back button. */
export function CanvasBreadcrumb({
  level,
  onNavigate,
}: {
  level: CanvasLevel;
  onNavigate: (level: CanvasLevel) => void;
}) {
  const cc = useContent().dashboard.canvas;
  const parent = parentLevel(level);

  const crumbs: { key: string; label: string; target: CanvasLevel | null }[] = [
    {
      key: 'root',
      label: cc.breadcrumbRoot,
      target: level.kind === 'root' ? null : { kind: 'root' },
    },
  ];
  if (level.kind === 'account') {
    crumbs.push({ key: 'account', label: level.login, target: null });
  } else if (level.kind === 'repo') {
    crumbs.push({
      key: 'account',
      label: level.owner,
      target: { kind: 'account', login: level.owner },
    });
    crumbs.push({ key: 'repo', label: `${level.owner}/${level.name}`, target: null });
  }

  return (
    <nav
      aria-label={cc.breadcrumbAria}
      // Elevated glass crumb bar: frosted translucent pill with a hairline
      // border and a soft inner top highlight, so the trail floats above the
      // canvas as its own surface. Layout (flex-wrap + min-h) is unchanged.
      className="flex items-center gap-1 flex-wrap min-h-[34px] bg-glass backdrop-blur-glass border border-line rounded-control px-2 shadow-[var(--shadow-1),var(--highlight-top)]"
    >
      {parent != null && (
        <button
          type="button"
          onClick={() => onNavigate(parent)}
          aria-label={cc.backAria}
          // Secondary glass button; hover warms the text and blooms a subtle
          // amber glow, echoing the Controls cluster on the canvas.
          className="font-ui font-semibold text-[12px] bg-glass-2 border border-line rounded-control px-2.5 py-1 text-dim hover:text-fg hover:border-line-2 hover:shadow-glow-amber transition-[color,border-color,box-shadow] cursor-pointer mr-2"
        >
          {cc.back}
        </button>
      )}
      {crumbs.map((crumb, i) => (
        <span key={crumb.key} className="flex items-center gap-1">
          {i > 0 && (
            <span className="font-mono text-[12px] text-ghost" aria-hidden="true">
              /
            </span>
          )}
          {crumb.target != null ? (
            <button type="button" onClick={() => onNavigate(crumb.target!)} className={crumbButton}>
              {crumb.label}
            </button>
          ) : (
            // Current level: brand amber→gold gradient accent (clipped into the
            // text) marks "you are here" without color alone carrying meaning —
            // aria-current="page" is the semantic signal.
            <span
              aria-current="page"
              className="grad-text font-mono font-semibold text-[12px] px-1.5 py-1"
            >
              {crumb.label}
            </span>
          )}
        </span>
      ))}
      {parent != null && (
        <span className="font-mono text-[10.5px] text-ghost ml-2" aria-hidden="true">
          {cc.escHint}
        </span>
      )}
    </nav>
  );
}
