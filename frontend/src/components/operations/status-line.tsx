import { useContent, useLang } from '@/i18n';
import type { SourceStatus } from '@/lib/api/operations';
import { SANDBOX_WARNINGS, SOURCE_HEALTHS, SOURCE_MESSAGES, asMember } from '@/lib/api/operations';
import { formatAbsolute, formatLocal } from '@/lib/format';
import { StatusPill } from './parts';

/**
 * The freshness / delivery status line.
 *
 * The distinction it exists to preserve: a page that is COMPLETE and empty, a
 * page that is PARTIAL because a source could not answer, and a snapshot that is
 * STALE because the feed stopped are three different facts, and only one of them
 * means "there is nothing to see". Collapsing them — a spinner over an empty
 * table, say — is the failure mode this whole surface is built against.
 *
 * Source health is DEPLOYMENT health. It never carries a count, a ratio, or any
 * other statistic derived from records the caller is not authorized to see.
 */
export function ActivityStatusLine({
  status,
  queriedAt,
}: {
  status: SourceStatus;
  queriedAt: string;
}) {
  const t = useContent().operations;
  const { lang } = useLang();
  const message = asMember(SOURCE_MESSAGES, status.message_code);

  return (
    <div
      data-testid="activity-status-line"
      className="flex-none flex items-center gap-x-3 gap-y-1 flex-wrap font-mono text-[10.5px] text-ghost"
    >
      <span title={formatAbsolute(queriedAt, lang)}>
        {t.queriedAt.replace('{time}', formatLocal(queriedAt, lang))}
      </span>
      <span aria-hidden="true">·</span>
      <span className="flex items-center gap-1.5">
        <span>{t.sourcesLabel}</span>
        <SourceChip label={t.posthogLabel} health={status.posthog} />
        <SourceChip label={t.relayLabel} health={status.relay} />
      </span>
      {status.partial && (
        <span data-testid="activity-partial" className="text-warn">
          {message ? t.sourceMessage[message] : t.partialNotice}
        </span>
      )}
    </div>
  );
}

function SourceChip({ label, health }: { label: string; health: string }) {
  const t = useContent().operations;
  const known = asMember(SOURCE_HEALTHS, health);
  const tone =
    known === 'healthy'
      ? 'green'
      : known === 'unavailable'
        ? 'red'
        : known === 'degraded'
          ? 'amber'
          : 'neutral';
  return (
    <StatusPill tone={tone}>
      {label}: {known ? t.sourceHealth[known] : health}
    </StatusPill>
  );
}

/** The sandbox snapshot's own freshness line, plus any snapshot-level warnings.
 *  `stale` is decided against the BACKEND's `observed_at`, never the moment the
 *  browser received the response. */
export function SandboxStatusLine({
  observedAt,
  stale,
  warningCodes,
}: {
  observedAt: string;
  stale: boolean;
  warningCodes: string[];
}) {
  const t = useContent().operations;
  const { lang } = useLang();
  return (
    <div
      data-testid="sandbox-status-line"
      className="flex-none flex items-center gap-x-3 gap-y-1 flex-wrap font-mono text-[10.5px] text-ghost"
    >
      <span title={formatAbsolute(observedAt, lang)}>
        {t.observedAt.replace('{time}', formatLocal(observedAt, lang))}
      </span>
      {stale && (
        <span data-testid="sandbox-stale" className="text-warn">
          {t.staleNotice}
        </span>
      )}
      {warningCodes.length > 0 && (
        <span className="flex items-center gap-1 flex-wrap">
          <span>{t.inventoryWarnings}</span>
          {warningCodes.map((code) => {
            const warning = asMember(SANDBOX_WARNINGS, code);
            return (
              <StatusPill key={code} tone="amber">
                {warning ? t.warning[warning] : code}
              </StatusPill>
            );
          })}
        </span>
      )}
    </div>
  );
}
