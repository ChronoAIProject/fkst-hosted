import type { Lang } from '@/i18n';

/** Format an epoch-ms as Singapore time (the dashboard's canonical timezone). */
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

/** Format an ISO timestamp (the GitHub wire format) as SGT; null-safe. */
export function formatIsoSgt(iso: string | null, lang: Lang): string | null {
  if (!iso) return null;
  const ms = Date.parse(iso);
  if (Number.isNaN(ms)) return null;
  return formatSgt(ms, lang);
}
