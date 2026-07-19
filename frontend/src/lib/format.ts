import type { Lang } from '@/i18n';

/** A timestamp accepted by every formatter: epoch-ms, or an ISO/wire string. */
export type TimeInput = number | string;

/**
 * Map a UI language to the BCP-47 locale used for date/number formatting.
 * Locale governs *formatting* (digit grouping, month order, unit words), NOT
 * the timezone — the timezone is always the viewer's, unless a formatter is
 * explicitly told otherwise (see the deprecated SGT helpers below).
 */
function locale(lang: Lang): string {
  return lang === 'zh' ? 'zh-CN' : 'en-GB';
}

/** Normalize any accepted input to epoch-ms; NaN signals an unparseable value. */
function toMs(value: TimeInput): number {
  return typeof value === 'number' ? value : Date.parse(value);
}

/**
 * Format a timestamp in the VIEWER's local timezone — the default for anything
 * shown to a human. No fixed timezone is imposed, so no mental UTC±N
 * conversion is required of the reader (unlike the deprecated SGT helpers).
 */
export function formatLocal(value: TimeInput, lang: Lang): string {
  const ms = toMs(value);
  if (Number.isNaN(ms)) return typeof value === 'string' ? value : '';
  try {
    return new Intl.DateTimeFormat(locale(lang), {
      dateStyle: 'medium',
      timeStyle: 'short',
    }).format(new Date(ms));
  } catch {
    // Intl can throw only on an out-of-range Date; surface a machine value.
    return new Date(ms).toISOString();
  }
}

/**
 * Format the full, unambiguous absolute value — including the timezone name —
 * intended for a `title` tooltip that backs a terser relative/local display.
 * `timeStyle: 'long'` carries the zone (e.g. "GMT+8") so the tooltip stands on
 * its own regardless of where the viewer is.
 */
export function formatAbsolute(value: TimeInput, lang: Lang): string {
  const ms = toMs(value);
  if (Number.isNaN(ms)) return typeof value === 'string' ? value : '';
  try {
    return new Intl.DateTimeFormat(locale(lang), {
      dateStyle: 'medium',
      timeStyle: 'long',
    }).format(new Date(ms));
  } catch {
    return new Date(ms).toISOString();
  }
}

/** Coarsest-first ranges for relative bucketing; `unit` feeds RelativeTimeFormat. */
const RELATIVE_RANGES: ReadonlyArray<{ unit: Intl.RelativeTimeFormatUnit; ms: number }> = [
  { unit: 'year', ms: 31_536_000_000 },
  { unit: 'month', ms: 2_592_000_000 },
  { unit: 'week', ms: 604_800_000 },
  { unit: 'day', ms: 86_400_000 },
  { unit: 'hour', ms: 3_600_000 },
  { unit: 'minute', ms: 60_000 },
];

/** Anything closer than this to `now` reads as "just now" rather than "0 min". */
const JUST_NOW_MS = 60_000;

/**
 * Format a timestamp as a compact relative bucket relative to `now` — past
 * ("2 min ago"), future ("in 3 hr"), and a localized "just now" for events
 * within the last/next minute. `now` is injectable so callers (and tests) can
 * pin the reference instant; it defaults to the wall clock.
 *
 * The unit words are localized via Intl.RelativeTimeFormat, so zh renders
 * "2分钟前" / "现在" without any extra i18n strings.
 */
export function formatRelative(value: TimeInput, lang: Lang, now: number = Date.now()): string {
  const ms = toMs(value);
  // Fall back to the absolute local render rather than emit a broken "NaN ago".
  if (Number.isNaN(ms)) return formatLocal(value, lang);

  const deltaMs = ms - now;
  if (Math.abs(deltaMs) < JUST_NOW_MS) {
    // numeric:'auto' renders the zero-second case as the localized "now".
    return new Intl.RelativeTimeFormat(locale(lang), { numeric: 'auto' }).format(0, 'second');
  }

  const rtf = new Intl.RelativeTimeFormat(locale(lang), { numeric: 'always', style: 'short' });
  for (const range of RELATIVE_RANGES) {
    if (Math.abs(deltaMs) >= range.ms) {
      // Sign of deltaMs is preserved: negative → "ago", positive → "in …".
      return rtf.format(Math.round(deltaMs / range.ms), range.unit);
    }
  }
  // Unreachable (JUST_NOW_MS equals the smallest range), but keep it total.
  return new Intl.RelativeTimeFormat(locale(lang), { numeric: 'auto' }).format(0, 'second');
}

/**
 * @deprecated Forces Asia/Singapore + an "SGT" suffix on every viewer, making
 * out-of-SGT readers convert in their head. Prefer {@link formatLocal} /
 * {@link formatRelative} (with {@link formatAbsolute} for the tooltip). Kept so
 * existing re-export/import paths (dashboard.tsx) stay green.
 */
export function formatSgt(ms: number, lang: Lang): string {
  try {
    const s = new Intl.DateTimeFormat(lang === 'zh' ? 'zh-CN' : 'en-GB', {
      timeZone: 'Asia/Singapore',
      dateStyle: 'medium',
      timeStyle: 'short',
    }).format(new Date(ms));
    return `${s} SGT`;
  } catch {
    return new Date(ms).toISOString();
  }
}

/**
 * @deprecated ISO-string wrapper over {@link formatSgt}; same SGT-forcing
 * caveat. Prefer {@link formatLocal}, which already accepts an ISO string.
 * Null-safe; retained for the current session-card call sites.
 */
export function formatIsoSgt(iso: string | null, lang: Lang): string | null {
  if (!iso) return null;
  const ms = Date.parse(iso);
  if (Number.isNaN(ms)) return null;
  return formatSgt(ms, lang);
}
