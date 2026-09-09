import type { ReactNode } from 'react';
import { cn } from '@/lib/utils';

/**
 * The app's one in-flight indicator: a small rotating ring.
 *
 * `.anim-spin` (index.css) rather than Tailwind's `animate-spin`, which is
 * banned by the lint rules — and, more to the point, `.anim-spin` is already
 * collapsed to a static ring by the global `prefers-reduced-motion` block, so
 * reduced motion is honoured without any call site opting in.
 *
 * It is `aria-hidden` deliberately: a spinning ring says nothing. Every wait
 * must carry a visible, announced LABEL — which is what [`LoadingState`]
 * exists to make the easy path, and why under reduced motion that label is the
 * only thing left conveying the state.
 */
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

/**
 * The shared in-flight state: a spinner, a label saying what is happening, and
 * an optional second line explaining WHY it is taking time.
 *
 * The `detail` line is the point of this component. Several of the app's waits
 * are a live GitHub fan-out or a pod exec, where seconds are normal and a bare
 * spinner leaves the reader guessing whether anything is wrong. The explanation
 * belongs beside the label, once, rather than being re-derived per call site.
 *
 * Announced through `role="status"` (the convention already used by this
 * project's skeletons), so the wait reaches assistive technology and not only
 * the eye. Pass `announce={false}` when the caller ALREADY carries
 * `role="status"` — nesting two live regions double-announces and breaks
 * `getByRole('status')` queries.
 */
export function LoadingState({
  label,
  detail,
  variant = 'inline',
  announce = true,
  className,
  testId,
}: {
  /** What is happening, in the surface's own words. Always visible. */
  label: ReactNode;
  /** Why it may take a moment. Omit for waits that are genuinely instant. */
  detail?: ReactNode;
  /** `inline` sits in a flow of content; `block` centres itself in an empty
   *  region (a table body, a panel with nothing else in it yet). */
  variant?: 'inline' | 'block';
  /** Set false when an ancestor is already the live region. */
  announce?: boolean;
  className?: string;
  testId?: string;
}) {
  return (
    <div
      {...(announce ? { role: 'status' } : {})}
      data-testid={testId}
      className={cn(
        'flex flex-col gap-1.5 min-w-0',
        variant === 'block' && 'items-center justify-center text-center py-10 px-4',
        className
      )}
    >
      <span className="inline-flex items-center gap-2 font-mono text-[11.5px] text-dim min-w-0">
        <Spinner />
        <span className="min-w-0">{label}</span>
      </span>
      {detail && (
        <p
          className={cn(
            'font-mono text-[11.5px] text-ghost leading-relaxed',
            variant === 'block' && 'max-w-[36ch]'
          )}
        >
          {detail}
        </p>
      )}
    </div>
  );
}
