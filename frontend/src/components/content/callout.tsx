import React from 'react';
import { cn } from '@/lib/utils';

type Tone = 'note' | 'tip' | 'warn';

const TONES: Record<Tone, { edge: string; label: string; defaultTitle: string }> = {
  note: { edge: 'border-l-line-2', label: 'text-faint', defaultTitle: 'Note' },
  tip: { edge: 'border-l-green', label: 'text-green', defaultTitle: 'Tip' },
  warn: { edge: 'border-l-amber', label: 'text-amber', defaultTitle: 'Heads up' },
};

export interface CalloutProps {
  tone?: Tone;
  /** Overrides the tone's default label ("Note" / "Tip" / "Heads up"). */
  title?: string;
  children: React.ReactNode;
  className?: string;
}

/** A bordered aside with a tone-coloured left edge, for notes/tips/warnings. */
export function Callout({ tone = 'note', title, children, className }: CalloutProps) {
  const t = TONES[tone];
  return (
    <div
      className={cn(
        // Elevated aside: a translucent glass panel lifted on the card shadow
        // with an inner top highlight (fakes light on a raised edge); the
        // tone-coloured left edge + label still carry the note/tip/warn meaning.
        'border border-line border-l-2 rounded-card px-4 py-3.5 bg-glass backdrop-blur-glass shadow-[var(--shadow-2),var(--highlight-top)]',
        t.edge,
        className
      )}
    >
      <div
        className={cn(
          'font-mono text-[10.5px] uppercase tracking-[0.12em] font-semibold mb-1.5',
          t.label
        )}
      >
        {title ?? t.defaultTitle}
      </div>
      <div className="text-[13px] leading-relaxed text-dim [&_a]:text-fg [&_a]:underline [&_a]:decoration-line-2 hover:[&_a]:decoration-faint [&_code]:font-mono [&_code]:text-[12px] [&_code]:text-fg [&_code]:bg-raise-2 [&_code]:rounded-chip [&_code]:px-1 [&_code]:py-0.5">
        {children}
      </div>
    </div>
  );
}
