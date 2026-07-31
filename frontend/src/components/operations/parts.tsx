import type { ReactNode } from 'react';
import { AlertTriangle, RefreshCw } from 'lucide-react';
import { cn } from '@/lib/utils';
import { EMPTY_VALUE } from '@/lib/operations/format';

/**
 * The small, repeated pieces of the operations workspace.
 *
 * Two rules are encoded here rather than repeated at every call site.
 *
 * **A value that does not exist renders the same dash everywhere.** A blank cell
 * reads as a rendering bug; an em dash reads as "there is nothing here", which is
 * a fact the audit surface is allowed to state.
 *
 * **Long values truncate visually and stay reachable.** Every truncating cell
 * carries the full value in its `title`, and the table's own layout is fixed —
 * so a longer operation id on the next poll changes no column width. Columns
 * that jump under a refresh are the single fastest way to make a dense table
 * unusable.
 */

/** A value that legitimately does not exist. */
export function Absent() {
  return (
    <span aria-hidden="true" className="text-ghost">
      {EMPTY_VALUE}
    </span>
  );
}

/** A monospaced cell whose overflow is clipped, with the full value available to
 *  pointer (title) and assistive tech (the text node itself is complete). */
export function Truncated({
  value,
  className,
}: {
  value: string | null | undefined;
  className?: string;
}) {
  if (!value) return <Absent />;
  return (
    <span title={value} className={cn('block truncate', className)}>
      {value}
    </span>
  );
}

/** An uppercase mono eyebrow. */
export function Eyebrow({ children }: { children: ReactNode }) {
  return <span className="font-mono text-eyebrow text-ghost uppercase">{children}</span>;
}

/** A status pill whose meaning is carried by its TEXT; the tint is decorative
 *  reinforcement, never the only signal. Fixed height and tabular digits keep
 *  the column width stable across polls. */
export function StatusPill({
  tone,
  children,
  title,
}: {
  tone: 'neutral' | 'amber' | 'green' | 'red';
  children: ReactNode;
  title?: string;
}) {
  return (
    <span
      title={title}
      className={cn(
        'inline-flex items-center h-[18px] px-1.5 rounded-chip border font-mono text-[10.5px] whitespace-nowrap tabular-nums',
        tone === 'green' &&
          'text-green bg-[color-mix(in_oklab,var(--green)_12%,var(--raise-2))] border-[color-mix(in_oklab,var(--green)_40%,var(--line))]',
        tone === 'amber' &&
          'text-amber bg-[color-mix(in_oklab,var(--amber)_12%,var(--raise-2))] border-[color-mix(in_oklab,var(--amber)_40%,var(--line))]',
        tone === 'red' &&
          'text-red bg-[color-mix(in_oklab,var(--red)_12%,var(--raise-2))] border-[color-mix(in_oklab,var(--red)_40%,var(--line))]',
        tone === 'neutral' && 'text-faint bg-raise-2 border-line-2'
      )}
    >
      {children}
    </span>
  );
}

/** A quiet advisory line: glass fill, amber left rule, no motion of its own. */
export function Notice({
  children,
  testId,
  tone = 'amber',
}: {
  children: ReactNode;
  testId?: string;
  tone?: 'amber' | 'red';
}) {
  return (
    <p
      data-testid={testId}
      className={cn(
        'bg-glass backdrop-blur-glass border border-line border-l-2 rounded-card px-3 py-1.5 font-mono text-[11.5px] text-dim',
        tone === 'amber' ? 'border-l-amber' : 'border-l-red'
      )}
    >
      {children}
    </p>
  );
}

/** The non-spinning empty state. Distinct from every failure state by design:
 *  "there is nothing here" and "we could not find out" must never look alike. */
export function EmptyState({ message }: { message: string }) {
  return (
    <div
      data-testid="operations-empty"
      className="flex-1 min-h-[160px] flex items-center justify-center px-6 py-10"
    >
      <p className="font-mono text-[12px] text-ghost text-center max-w-[46ch]">{message}</p>
    </div>
  );
}

/** The failure state: a stable title, the localized code copy, the request id
 *  when one was exposed, and a retry. Never the backend's own message. */
export function ErrorState({
  title,
  message,
  requestId,
  requestIdLabel,
  retryLabel,
  onRetry,
}: {
  title: string;
  message: string;
  requestId: string | null;
  requestIdLabel: string;
  retryLabel: string;
  onRetry: () => void;
}) {
  return (
    <div
      role="alert"
      data-testid="operations-error"
      className="flex-1 min-h-[160px] flex items-center justify-center px-6 py-10"
    >
      <div className="grad-border rounded-card px-6 py-5 shadow-2 flex flex-col items-center gap-3 text-center max-w-[52ch]">
        <AlertTriangle aria-hidden="true" className="w-4 h-4 text-amber" />
        <p className="font-ui font-semibold text-[13px] text-fg">{title}</p>
        <p className="font-mono text-[11.5px] text-dim">{message}</p>
        {requestId && (
          <p className="font-mono text-[10.5px] text-ghost break-all">
            {requestIdLabel.replace('{id}', requestId)}
          </p>
        )}
        <button
          type="button"
          onClick={onRetry}
          className="font-ui font-semibold text-[12px] border border-line rounded-control px-3 py-1.5 text-fg hover:shadow-glow-amber transition-[box-shadow] cursor-pointer inline-flex items-center gap-1.5"
        >
          <RefreshCw aria-hidden="true" className="w-3 h-3" />
          {retryLabel}
        </button>
      </div>
    </div>
  );
}

/** The refresh action shared by both views. Icon-only styling is avoided: the
 *  label is visible AND the busy state is announced. */
export function RefreshButton({
  label,
  busyLabel,
  busy,
  onClick,
}: {
  label: string;
  busyLabel: string;
  busy: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-busy={busy}
      className="font-ui font-semibold text-[12px] border border-line rounded-control px-2.5 py-1.5 text-dim hover:text-fg hover:shadow-glow-amber transition-[color,box-shadow] cursor-pointer inline-flex items-center gap-1.5 flex-none"
    >
      <RefreshCw
        aria-hidden="true"
        className={cn('w-3 h-3', busy && 'anim-spin motion-reduce:animate-none')}
      />
      {busy ? busyLabel : label}
    </button>
  );
}
