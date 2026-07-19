import type { ReactNode } from 'react';
import { useContent } from '@/i18n';
import { Chip } from '@/components/ui/chip';
import type { SessionDetail } from '@/lib/api/types';
import { Note, SectionLabel } from './parts';

/** One definition row (label → value). The label column is `max-content` so
 *  every row aligns on a single value gutter regardless of label length. */
function ConfigRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <>
      <dt className="font-mono text-[11px] text-ghost self-baseline">{label}</dt>
      <dd className="text-[12.5px] text-fg min-w-0 break-words">{children}</dd>
    </>
  );
}

/** ConfigPanel: the FULL session configuration that is frozen at registration
 *  (work label, environment, auto-merge, output language, and the log-access
 *  allowlist). None of these are surfaced anywhere else in the UI today, so the
 *  detail drawer is the one place a viewer can confirm what a session was
 *  actually registered with. A scalar the session did not carry renders as a
 *  muted em-dash rather than a blank cell, and an empty log-access allowlist
 *  renders an explicit "none" so an unset allowlist is never confused with a
 *  failed render. */
export function ConfigPanel({ session }: { session: SessionDetail }) {
  const t = useContent().dashboard.detail;

  // A missing scalar is a real state (the trigger issue simply omitted the
  // field); show the frozen em-dash so the row still reads as configured-empty.
  const unset = <span className="text-ghost">{t.configUnset}</span>;

  const workLabel = session.work_label ? (
    <span className="font-mono text-[12px]">{session.work_label}</span>
  ) : (
    unset
  );

  const environment = session.environment ? (
    <span className="font-mono text-[12px]">{session.environment}</span>
  ) : (
    unset
  );

  const outputLang = session.output_lang ? (
    <span className="font-mono text-[12px]">{session.output_lang}</span>
  ) : (
    unset
  );

  // auto_merge is a tri-state: yes / no / not-recorded (null).
  const autoMerge =
    session.auto_merge === null || session.auto_merge === undefined
      ? unset
      : session.auto_merge
        ? t.configYes
        : t.configNo;

  // log_access is optional AND nullable on the wire; treat every empty shape
  // (undefined / null / []) as "no additional viewers".
  const logAccess = session.log_access ?? [];

  return (
    <section className="flex flex-col gap-2.5">
      <SectionLabel>{t.configLabel}</SectionLabel>
      <dl className="grid grid-cols-[max-content_1fr] gap-x-4 gap-y-2 min-w-0">
        <ConfigRow label={t.configWorkLabel}>{workLabel}</ConfigRow>
        <ConfigRow label={t.configEnvironment}>{environment}</ConfigRow>
        <ConfigRow label={t.configAutoMerge}>{autoMerge}</ConfigRow>
        <ConfigRow label={t.configOutputLang}>{outputLang}</ConfigRow>
        <ConfigRow label={t.configLogAccess}>
          {logAccess.length === 0 ? (
            <span className="text-ghost">{t.configLogAccessNone}</span>
          ) : (
            <span className="flex flex-wrap gap-1.5">
              {logAccess.map((viewer) => (
                <Chip key={viewer}>{viewer}</Chip>
              ))}
            </span>
          )}
        </ConfigRow>
      </dl>
      <Note>{t.configFrozenNote}</Note>
    </section>
  );
}
