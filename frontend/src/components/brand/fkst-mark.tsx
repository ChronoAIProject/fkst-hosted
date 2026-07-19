import { cn } from '@/lib/utils';

export interface FkstMarkProps {
  /** Size/weight/colour are driven by Tailwind text-* classes passed here. */
  className?: string;
}

/**
 * The FKST wordmark — the counter of the "K" carries an amber→gold accent dot.
 * The dot is sized in `em`, so it scales with whatever font-size the consumer
 * applies, and it wears the brand gradient so the mark stays consistent whether
 * the letters are a flat color or the consumer clips a `.grad-text` sweep over
 * them (as the shell topbar does).
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
          // The --bg ring keeps the dot legible against any letter fill; a soft
          // amber bloom gives it the same lit quality as the app's accent marks.
          style={{ boxShadow: '0 0 0 0.05em var(--bg), var(--glow-amber)' }}
          className="absolute left-[0.04em] top-[0.36em] w-[0.205em] h-[0.205em] rounded-full bg-grad-accent"
          aria-hidden="true"
        />
      </span>
      ST
    </span>
  );
}
