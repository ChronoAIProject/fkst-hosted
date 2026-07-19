import { useEffect, useRef, useState } from 'react';
import { motion, useReducedMotion } from 'framer-motion';
import { cn } from '@/lib/utils';

export interface CopyButtonProps {
  /** Exact text written to the clipboard, verbatim. */
  value: string;
  /**
   * Accessible action label (also the visible text). Consumers pass an
   * already-localized string; the English default is only a bare fallback.
   */
  label?: string;
  className?: string;
}

/** How long the transient "copied" confirmation stays up, in ms. */
const COPIED_HOLD_MS = 1500;

/**
 * Writes `text` to the clipboard, preferring the async Clipboard API and
 * falling back to a hidden-textarea `execCommand('copy')` for insecure
 * contexts / older engines where `navigator.clipboard` is absent. Returns
 * whether the copy actually succeeded so the caller can decide whether to
 * show the confirmation — a silent failure would be a lie to the user.
 */
async function writeToClipboard(text: string): Promise<boolean> {
  if (navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      // Permissions / insecure-context rejection — fall through to the
      // legacy path rather than leaving the user with nothing.
    }
  }

  // Legacy fallback: a transient off-screen textarea + execCommand. Guard the
  // whole thing because execCommand can throw and is missing in some jsdom /
  // sandboxed environments.
  try {
    const textarea = document.createElement('textarea');
    textarea.value = text;
    // Keep it out of the layout and unfocusable to the eye, but selectable.
    textarea.setAttribute('readonly', '');
    textarea.style.position = 'fixed';
    textarea.style.top = '-9999px';
    textarea.style.opacity = '0';
    document.body.appendChild(textarea);
    textarea.select();
    const ok = typeof document.execCommand === 'function' && document.execCommand('copy');
    document.body.removeChild(textarea);
    return Boolean(ok);
  } catch {
    return false;
  }
}

/**
 * Reusable compact copy affordance: an icon (plus optional text) styled like
 * the app's small chip-buttons, a ~1.5s "copied" state, and a polite live
 * region so screen-reader users hear the confirmation the sighted checkmark
 * conveys. Used for session ids, package refs, log filenames, etc.
 */
export function CopyButton({ value, label = 'Copy', className }: CopyButtonProps) {
  const [copied, setCopied] = useState(false);
  const reduceMotion = useReducedMotion();
  // Track the pending reset so re-clicks restart (not stack) the timer, and so
  // an unmount mid-hold cannot setState on a dead component.
  const resetTimer = useRef<number | null>(null);

  useEffect(
    () => () => {
      if (resetTimer.current !== null) {
        window.clearTimeout(resetTimer.current);
      }
    },
    []
  );

  const handleCopy = async () => {
    const ok = await writeToClipboard(value);
    // Only confirm on a real success — the fallback may still fail (e.g. no
    // execCommand), and claiming "copied" when nothing was copied misleads.
    if (!ok) return;

    setCopied(true);
    if (resetTimer.current !== null) {
      window.clearTimeout(resetTimer.current);
    }
    resetTimer.current = window.setTimeout(() => {
      setCopied(false);
      resetTimer.current = null;
    }, COPIED_HOLD_MS);
  };

  return (
    <button
      type="button"
      onClick={handleCopy}
      aria-label={label}
      className={cn(
        'inline-flex items-center gap-1.5 flex-none font-mono text-[10.5px]',
        'px-2 py-0.5 rounded-chip border border-line-2 bg-raise-2 text-faint',
        'hover:text-fg hover:border-faint transition-colors cursor-pointer',
        className
      )}
    >
      <motion.span
        aria-hidden="true"
        className="inline-flex"
        // A tiny pop on the icon swap; instant under reduced-motion so the
        // final state is never gated behind a skipped animation.
        key={copied ? 'check' : 'copy'}
        initial={reduceMotion ? false : { scale: 0.6, opacity: 0 }}
        animate={{ scale: 1, opacity: 1 }}
        transition={{ duration: reduceMotion ? 0 : 0.14 }}
      >
        {copied ? <CheckIcon /> : <CopyIcon />}
      </motion.span>
      <span>{label}</span>
      {/* Screen-reader-only confirmation. Empty until a copy lands, then the
          polite live region announces it without stealing focus. */}
      <span aria-live="polite" className="sr-only">
        {copied ? 'Copied' : ''}
      </span>
    </button>
  );
}

function CopyIcon() {
  return (
    <svg width="11" height="11" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5">
      <rect x="5.5" y="5.5" width="8" height="8" rx="1.5" />
      <path d="M10.5 3.5V3a1.5 1.5 0 0 0-1.5-1.5H4A1.5 1.5 0 0 0 2.5 3v5A1.5 1.5 0 0 0 4 9.5h.5" />
    </svg>
  );
}

function CheckIcon() {
  return (
    <svg
      width="11"
      height="11"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.75"
      className="text-green"
    >
      <path d="M3 8.5l3.5 3.5L13 5" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}
