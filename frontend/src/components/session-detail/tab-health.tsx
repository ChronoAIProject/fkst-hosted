import { useCallback, useEffect, useRef, useState } from 'react';
import { useContent, useLang } from '@/i18n';
import { useAuth } from '@/lib/auth/github-auth';
import { cn } from '@/lib/utils';
import { formatAbsolute, formatTimeShort } from '@/lib/format';
import { getHealthReport, type HealthReport, type SessionHealth } from '@/lib/api/health';
import { Chip } from '@/components/ui/chip';
import { MarkdownPreview } from '@/components/ui/markdown-preview';
import { ScrollArea } from '@/components/ui/scroll-area';
import { Note, SectionLabel, Spinner, SplitPanes } from './parts';
import { StatusCard } from './status-charts';
import { HEALTH_TONE, minutes, showsStaleNotice } from './health-state';

/** The health listing's load state, owned by the parent so the header chip and
 *  this tab share ONE fetch. */
export type HealthState =
  | { status: 'idle' | 'loading' }
  | { status: 'loaded'; health: SessionHealth }
  | { status: 'error'; httpStatus?: number };

type ReportState =
  | { status: 'idle' | 'loading' }
  | { status: 'loaded'; report: HealthReport }
  | { status: 'error' };

/**
 * The Health tab: the current business-aware assessment, the evidence behind it,
 * the producer's narrative, and the report history.
 *
 * Not gated on liveness, deliberately — reading the health history of a paused or
 * retired session is a primary use case, and the read is a cheap cached index
 * lookup rather than a runtime exec.
 */
export function TabHealth({
  sessionId,
  state,
  onRetry,
}: {
  sessionId: string;
  state: HealthState;
  onRetry: () => void;
}) {
  const t = useContent().dashboard.detail;
  const { lang } = useLang();
  const { apiFetch } = useAuth();

  const [selected, setSelected] = useState<string | null>(null);
  const [report, setReport] = useState<ReportState>({ status: 'idle' });
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const health = state.status === 'loaded' ? state.health : null;
  const latest = health?.latest ?? health?.reports[0] ?? null;
  // The newest report is what the tab opens on; a history click overrides it.
  const activeId = selected ?? latest?.id ?? null;

  const loadReport = useCallback(
    (reportId: string) => {
      if (!sessionId) return;
      setReport({ status: 'loading' });
      getHealthReport(apiFetch, sessionId, reportId)
        .then((loaded) => {
          if (mounted.current) setReport({ status: 'loaded', report: loaded });
        })
        .catch(() => {
          if (mounted.current) setReport({ status: 'error' });
        });
    },
    [apiFetch, sessionId]
  );

  useEffect(() => {
    if (activeId) loadReport(activeId);
  }, [activeId, loadReport]);

  if (state.status === 'idle' || state.status === 'loading') {
    return (
      <div className="flex items-center gap-2 text-ghost text-[12.5px]">
        <Spinner />
        {t.healthLoading}
      </div>
    );
  }

  if (state.status === 'error') {
    // 503 is a deployment fact, not a failure — say so calmly and offer no retry.
    if (state.httpStatus === 503) return <Note>{t.healthUnavailable}</Note>;
    return (
      <div className="flex flex-col gap-2 items-start">
        <Note>{t.healthError}</Note>
        <button
          type="button"
          onClick={onRetry}
          className="text-[12px] font-mono text-dim hover:text-fg underline underline-offset-2"
        >
          {t.healthRetry}
        </button>
      </div>
    );
  }

  const staleness = health!.staleness;
  const ageMinutes = minutes(staleness.age_secs);
  const expectedMinutes = minutes(staleness.expected_interval_secs);
  const reports = health!.reports;
  // The right pane reflects the SELECTED report, not always the newest one. The
  // summary renders immediately from the listing; the full report enriches it once
  // its fetch lands, so switching entries never blanks the pane.
  const activeSummary = reports.find((entry) => entry.id === activeId) ?? latest;
  const loaded = report.status === 'loaded' ? report.report : null;
  const detail = loaded && loaded.id === activeId ? loaded : null;

  if (!activeSummary) {
    return staleness.state === 'not_running' ? (
      <Note>{t.healthNotRunning}</Note>
    ) : (
      <Note>{t.healthNeverReported}</Note>
    );
  }

  return (
    <div className="flex flex-col gap-3 h-full min-h-0">
      {/* Session-level heartbeat line: about the SESSION, not the selected report,
          so it sits above the master/detail split rather than inside either pane. */}
      <div className="flex items-center gap-2 flex-wrap text-[11.5px] font-mono text-ghost">
        <span>{t.healthStaleness[staleness.state]}</span>
        <span aria-hidden="true">·</span>
        <span>
          {ageMinutes == null
            ? t.healthLastReportUnknown
            : t.healthLastReport.replace('{n}', String(ageMinutes))}
        </span>
      </div>

      {/* Only when stale. Never for not_running, which is the normal end of a
          session's work. Session-level, so it stays above the split. */}
      {showsStaleNotice(health) && (
        <div
          role="status"
          className="rounded-card border border-[color-mix(in_oklab,var(--amber)_40%,var(--line))] bg-[color-mix(in_oklab,var(--amber)_8%,var(--raise-2))] px-3.5 py-3 flex flex-col gap-1"
        >
          <p className="text-[12.5px] text-amber font-medium">{t.healthStaleTitle}</p>
          <p className="text-[12px] text-dim leading-[1.55]">
            {t.healthStaleBody
              .replace('{expected}', String(expectedMinutes ?? '?'))
              .replace('{age}', String(ageMinutes ?? '?'))}
          </p>
        </div>
      )}

      {/* Both panes fill the SAME box and each scrolls its own content. Sizing to
          content instead leaves the columns unequal and lets the right pane's
          overflow escape to the tab panel — which would scroll the rail out of
          view while reading an entry. The height comes from the panel now, not a
          hardcoded viewport fraction, so every tab is the same size. */}
      <SplitPanes
        start={
          /* ---- master: one entry per report, newest first, keyed by time ---- */
          <nav className="flex flex-col gap-1.5 min-w-0 min-h-0">
            <SectionLabel>{t.healthHistory}</SectionLabel>
            <ScrollArea className="pr-1 max-h-[14rem] md:max-h-none">
              <ul aria-label={t.healthHistoryAria} className="flex flex-col gap-1">
                {reports.map((entry) => {
                  const active = entry.id === activeId;
                  return (
                    <li key={entry.id}>
                      {/* Stacked, not side by side: a stamp and a chip on one 11.5rem row
                      forces one of them to truncate, and the truncated part of a
                      timestamp is exactly what tells two entries apart. */}
                      <button
                        type="button"
                        aria-current={active}
                        title={formatAbsolute(entry.generated_at, lang)}
                        onClick={() => setSelected(entry.id)}
                        className={cn(
                          'w-full text-left rounded-control border px-2.5 py-1.5',
                          'flex flex-col items-start gap-1 min-w-0',
                          active ? 'border-line-2 bg-raise-2' : 'border-line hover:bg-raise-1'
                        )}
                      >
                        <span className="text-[11px] font-mono text-dim">
                          {formatTimeShort(entry.generated_at, lang)}
                        </span>
                        <Chip tone={HEALTH_TONE[entry.status] ?? 'neutral'}>
                          {t.healthStatus[entry.status] ?? entry.status_raw}
                        </Chip>
                      </button>
                    </li>
                  );
                })}
              </ul>
            </ScrollArea>
          </nav>
        }
        end={
          /* ---- detail: everything about the selected report ---- */
          <ScrollArea className="pr-1">
            <section aria-label={t.healthDetailAria} className="flex flex-col gap-3.5 min-w-0">
              <StatusCard label={t.healthCurrent}>
                <div className="flex items-center gap-2 flex-wrap">
                  <Chip tone={HEALTH_TONE[activeSummary.status] ?? 'neutral'}>
                    {t.healthStatus[activeSummary.status] ?? activeSummary.status_raw}
                  </Chip>
                  <span
                    className="text-[11.5px] font-mono text-ghost"
                    title={formatAbsolute(activeSummary.generated_at, lang)}
                  >
                    {formatTimeShort(activeSummary.generated_at, lang)}
                  </span>
                </div>
                <p className="text-[13px] text-fg leading-[1.55] break-words">
                  {activeSummary.headline}
                </p>
                <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-[11.5px] font-mono text-ghost">
                  <dt>{t.healthProducer}</dt>
                  <dd className="text-dim break-all">{activeSummary.producer}</dd>
                  {detail?.confidence ? (
                    <>
                      <dt>{t.healthConfidence}</dt>
                      <dd className="text-dim">{detail.confidence}</dd>
                    </>
                  ) : null}
                </dl>
              </StatusCard>

              {report.status === 'loading' && (
                <div className="flex items-center gap-2 text-ghost text-[12.5px]">
                  <Spinner />
                  {t.healthLoading}
                </div>
              )}
              {report.status === 'error' && <Note>{t.healthError}</Note>}

              {detail && detail.evidence.length > 0 && (
                <div className="flex flex-col gap-1.5">
                  <SectionLabel>{t.healthEvidence}</SectionLabel>
                  <dl className="grid grid-cols-[minmax(0,auto)_minmax(0,1fr)] gap-x-3 gap-y-1 text-[11.5px] font-mono">
                    {detail.evidence.map((item) => (
                      <div key={item.key} className="contents">
                        <dt className="text-ghost break-all">{item.key}</dt>
                        <dd className="text-dim break-all">{item.value}</dd>
                      </div>
                    ))}
                  </dl>
                </div>
              )}

              {/* UNTRUSTED: authored by an LLM inside a session pod. MarkdownPreview emits
              React elements only (never raw HTML) and protocol-allowlists links. */}
              {detail && detail.body_markdown.trim().length > 0 && (
                <div className="flex flex-col gap-1.5">
                  <SectionLabel>{t.healthBody}</SectionLabel>
                  <MarkdownPreview
                    markdown={detail.body_markdown}
                    ariaLabel={t.healthBodyAria}
                    variant="flow"
                  />
                </div>
              )}
            </section>
          </ScrollArea>
        }
      />
    </div>
  );
}
