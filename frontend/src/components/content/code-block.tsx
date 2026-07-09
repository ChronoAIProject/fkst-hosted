import { useState } from 'react';
import { cn } from '@/lib/utils';

export interface CodeBlockProps {
  /** Raw text to render verbatim inside the block. */
  code: string;
  /** Small mono caption shown in the block's header bar. */
  caption?: string;
  className?: string;
}

/**
 * A monospace code/config block matching the console surface — raised card,
 * a header bar carrying a caption + copy affordance, and a horizontally
 * scrollable body so long lines never push the page wider than the viewport.
 */
export function CodeBlock({ code, caption, className }: CodeBlockProps) {
  const [copied, setCopied] = useState(false);

  const handleCopy = async () => {
    try {
      await navigator.clipboard?.writeText(code);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1400);
    } catch {
      // Clipboard may be unavailable (insecure context / permissions) — no-op.
    }
  };

  return (
    <div
      className={cn(
        'border border-line rounded-card bg-raise overflow-hidden min-w-0',
        className
      )}
    >
      <div className="flex items-center justify-between gap-2 px-3.5 py-2 border-b border-line bg-[color-mix(in_oklab,var(--raise-2)_55%,transparent)]">
        <span className="font-mono text-[11px] text-ghost uppercase tracking-[0.08em] truncate min-w-0">
          {caption ?? 'example'}
        </span>
        <button
          type="button"
          onClick={handleCopy}
          className="flex-none font-mono text-[10.5px] px-2 py-0.5 rounded-chip border border-line-2 bg-raise-2 text-faint hover:text-fg hover:border-faint transition-colors cursor-pointer"
        >
          {copied ? 'copied ✓' : 'copy'}
        </button>
      </div>
      <pre className="overflow-x-auto px-3.5 py-3 text-[12.5px] leading-[1.65] font-mono text-dim m-0">
        <code>{code}</code>
      </pre>
    </div>
  );
}
