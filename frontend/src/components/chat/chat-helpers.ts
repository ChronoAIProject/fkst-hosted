import { useContent } from '@/i18n';

/**
 * Small pure helpers shared by the chat context and its hooks.
 *
 * Extracted so `chat-context.tsx` stays under the repo's 500-line limit; both of
 * these are self-contained enough that they read better beside their own tests
 * than buried in the provider.
 */

let messageSeq = 0;
/** Monotonic ids. A counter, not a timestamp: two messages appended in the same
 *  millisecond must not collide as React keys. */
export function nextId(prefix: string): string {
  messageSeq += 1;
  return `${prefix}-${messageSeq}`;
}

/** Resolve user-facing error copy from a stable code.
 *
 *  `rate_limited` gets a variant naming the retry delay when the server sent one,
 *  because "try again in 5s" is actionable where "try again" is not. */
export function errorCopy(
  s: ReturnType<typeof useContent>['chat'],
  code: string,
  fallback: string,
  retryAfterSeconds?: number
): string {
  if (code === 'rate_limited' && retryAfterSeconds != null) {
    return s.errors.rate_limited_after!.replace('{seconds}', String(retryAfterSeconds));
  }
  return s.errors[code] ?? fallback ?? s.errors.unknown!;
}

