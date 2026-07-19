import { useId, useState } from 'react';
import { cn } from '@/lib/utils';
import { useContent } from '@/i18n';
import { Reveal } from '@/components/ui/motion';

const rows = [
  { key: 'none', dot: 'bg-line-2' },
  { key: 'installed', dot: 'bg-amber' },
  { key: 'active', dot: 'bg-amber anim-dot-blink' },
] as const;

/** The three-status legend shown on every sidebar level, so the canvas's
 *  color/motion language is always explained in plain words.
 *
 *  The three rows re-render identically on every level while the panel is
 *  height-constrained, so the legend is a self-contained disclosure: the
 *  "Legend" header is a toggle and the rows collapse away, letting the primary
 *  content (the session list) sit higher. It defaults CLOSED so that lift
 *  applies out of the box; the header text keeps the colour language named even
 *  when the swatches are hidden. State is internal, so every existing caller
 *  (`StatusLegend` with no props) gets the behaviour without changing. */
export function StatusLegend() {
  const cc = useContent().dashboard.canvas;
  const [open, setOpen] = useState(false);
  const bodyId = useId();
  const label = {
    none: cc.legendNone,
    installed: cc.legendInstalled,
    active: cc.legendActive,
  } as const;

  return (
    <div className="border border-line rounded-card bg-bg px-3 py-2.5 flex flex-col gap-1.5">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        aria-controls={bodyId}
        className="flex items-center gap-2 text-left cursor-pointer group"
      >
        <span
          aria-hidden="true"
          className={cn('font-mono text-[10px] text-ghost flex-none transition-transform', open && 'rotate-90')}
        >
          ▸
        </span>
        <span className="font-mono text-eyebrow text-ghost uppercase group-hover:text-dim transition-colors">
          {cc.legendTitle}
        </span>
      </button>
      <Reveal open={open}>
        <ul id={bodyId} className="flex flex-col gap-1 pt-0.5">
          {rows.map((row) => (
            <li key={row.key} className="flex items-center gap-2">
              <span aria-hidden="true" className={cn('w-2 h-2 rounded-full flex-none', row.dot)} />
              <span className="text-[11.5px] text-dim leading-snug">{label[row.key]}</span>
            </li>
          ))}
        </ul>
      </Reveal>
    </div>
  );
}

/** Level lede: states plainly what the current view represents. */
export function ViewDescription({ text }: { text: string }) {
  return <p className="text-[12.5px] leading-relaxed text-dim">{text}</p>;
}
