import { cn } from '@/lib/utils';

export interface FkstMarkProps {
  /** Size/weight/colour are driven by Tailwind text-* classes passed here. */
  className?: string;
}

/**
 * The FKST wordmark — the counter of the "K" carries an amber dot. The dot is
 * sized in `em`, so it scales with whatever font-size the consumer applies.
 */
export function FkstMark({ className }: FkstMarkProps) {
  return (
    <span
      className={cn(
        'font-display font-bold tracking-[0.01em] leading-none inline-block whitespace-nowrap select-none',
        className
      )}
    >
      F
      <span className="relative inline-block">
        K
        <span
          style={{ boxShadow: '0 0 0 0.05em var(--bg)' }}
          className="absolute left-[0.04em] top-[0.36em] w-[0.205em] h-[0.205em] rounded-full bg-amber"
          aria-hidden="true"
        />
      </span>
      ST
    </span>
  );
}
