import type { ReactNode } from 'react';
import { X } from 'lucide-react';
import { CopyButton } from '@/components/ui/copy-button';
import { Eyebrow } from './parts';

/**
 * The row-details surface: a side panel on wide viewports, a full-width bottom
 * drawer under 1100px. It is the same component in both cases — only the
 * positioning classes differ — so the content, the heading, and the close
 * affordance cannot drift apart between layouts.
 *
 * It is a labelled `complementary` region rather than a modal dialog on purpose:
 * the table stays live and readable beside it, which is the whole point of an
 * investigation surface. Nothing in it is interactive except Copy and Close, so
 * there is no focus trap to justify.
 */
export function DetailsPanel({
  title,
  ariaLabel,
  closeLabel,
  onClose,
  children,
}: {
  title: string;
  ariaLabel: string;
  closeLabel: string;
  onClose: () => void;
  children: ReactNode;
}) {
  return (
    <aside
      aria-label={ariaLabel}
      data-testid="operations-details"
      // Narrow widths get `flex-1` plus a `vh` cap rather than `flex-none` plus
      // a `%` one, and both halves of that matter. A percentage max-height
      // resolves against a containing block that is itself an indefinite-height
      // flex item here, so it silently computes to `none`; and a `flex-none`
      // panel cannot shrink when the wrapped filter toolbar has already eaten
      // most of a phone screen. Either alone leaves the panel overflowing the
      // fixed-height route — which is how a workspace that must never scroll the
      // document ends up scrolling it.
      className="flex-none w-[360px] min-h-0 overflow-y-auto border border-line rounded-panel bg-raise max-[1100px]:w-full max-[1100px]:flex-1 max-[1100px]:max-h-[45vh]"
    >
      <div className="sticky top-0 bg-glass backdrop-blur-glass border-b border-line px-3 py-2 flex items-center gap-2">
        <h3 className="font-ui font-semibold text-[12.5px] text-fg truncate min-w-0 flex-1">
          {title}
        </h3>
        <button
          type="button"
          onClick={onClose}
          aria-label={closeLabel}
          className="inline-flex items-center justify-center w-6 h-6 flex-none rounded-control text-faint hover:text-fg hover:bg-raise-2 transition-colors cursor-pointer"
        >
          <X aria-hidden="true" className="w-3.5 h-3.5" />
        </button>
      </div>
      <div className="px-3 py-3 flex flex-col gap-4">{children}</div>
    </aside>
  );
}

/** A titled group of fields. */
export function DetailSection({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-1.5">
      <Eyebrow>{title}</Eyebrow>
      <dl className="flex flex-col gap-1">{children}</dl>
    </section>
  );
}

/**
 * One field. Rows whose value is absent are dropped entirely rather than shown
 * as a dash: in a details panel a long list of dashes buries the handful of
 * values that actually exist.
 *
 * `copy` adds the shared copy affordance, whose accessible name the caller
 * supplies (an icon-only control with no name is unusable by screen reader).
 */
export function DetailField({
  label,
  value,
  copyLabel,
}: {
  label: string;
  value: string | number | null | undefined;
  copyLabel?: string;
}) {
  if (value === null || value === undefined || value === '') return null;
  const text = String(value);
  return (
    <div className="flex items-baseline gap-2 min-w-0">
      <dt className="font-mono text-[10px] text-ghost flex-none w-[104px]">{label}</dt>
      <dd className="font-mono text-[11px] text-dim min-w-0 flex-1 break-all">{text}</dd>
      {copyLabel && <CopyButton value={text} label={copyLabel} className="flex-none" />}
    </div>
  );
}
