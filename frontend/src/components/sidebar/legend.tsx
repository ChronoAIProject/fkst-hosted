import { cn } from '@/lib/utils';
import { useContent } from '@/i18n';

const rows = [
  { key: 'none', dot: 'bg-line-2' },
  { key: 'installed', dot: 'bg-amber' },
  { key: 'active', dot: 'bg-amber anim-dot-blink' },
] as const;

/** The three-status legend shown on every sidebar level, so the canvas's
 *  color/motion language is always explained in plain words. */
export function StatusLegend() {
  const cc = useContent().dashboard.canvas;
  const label = {
    none: cc.legendNone,
    installed: cc.legendInstalled,
    active: cc.legendActive,
  } as const;

  return (
    <div className="border border-line rounded-card bg-bg px-3 py-2.5 flex flex-col gap-1.5">
      <span className="font-mono text-eyebrow text-ghost uppercase">{cc.legendTitle}</span>
      <ul className="flex flex-col gap-1">
        {rows.map((row) => (
          <li key={row.key} className="flex items-center gap-2">
            <span aria-hidden="true" className={cn('w-2 h-2 rounded-full flex-none', row.dot)} />
            <span className="text-[11.5px] text-dim leading-snug">{label[row.key]}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

/** Level lede: states plainly what the current view represents. */
export function ViewDescription({ text }: { text: string }) {
  return <p className="text-[12.5px] leading-relaxed text-dim">{text}</p>;
}
