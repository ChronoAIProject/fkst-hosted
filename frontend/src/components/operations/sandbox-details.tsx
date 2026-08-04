import { Activity } from 'lucide-react';
import { useContent } from '@/i18n';
import type { SandboxRow } from '@/lib/api/operations';
import {
  ATTRIBUTION_SOURCES,
  SANDBOX_BACKENDS,
  SANDBOX_STATUSES,
  SANDBOX_WARNINGS,
  asMember,
} from '@/lib/api/operations';
import { displayLocation, formatDurationSeconds } from '@/lib/operations/format';
import { DetailField, DetailSection, DetailsPanel } from './details-panel';
import { StatusPill } from './parts';

/**
 * The sandbox row details, plus the one cross-link this surface offers.
 *
 * **View activity** switches to the Activity view filtered to this session with
 * `record_kind=all`. For a global administrator that is the session's whole
 * timeline. For a regular user it is exactly their OWN calls plus the system
 * lifecycle rows — never another collaborator's calls — because the server
 * applies that predicate, not this button. The button is a navigation
 * convenience; the semantics are the API's.
 *
 * The raw backend state is shown here (never in place of the normalized status),
 * and `backend_location` is passed through `displayLocation`, which cannot emit
 * anything that reads as a URL.
 */
export function SandboxDetails({
  row,
  onClose,
  onViewActivity,
}: {
  row: SandboxRow;
  onClose: () => void;
  /** Supplied only when the row carries a session id — an orphan runtime has no
   *  timeline to open. */
  onViewActivity: ((sessionId: string) => void) | null;
}) {
  const t = useContent().operations;
  const status = asMember(SANDBOX_STATUSES, row.status);
  const backend = asMember(SANDBOX_BACKENDS, row.backend);
  const attribution = asMember(ATTRIBUTION_SOURCES, row.attribution_source);
  const metadata =
    row.metadata_state in t.metadataState
      ? t.metadataState[row.metadata_state as keyof typeof t.metadataState]
      : row.metadata_state;

  return (
    <DetailsPanel
      title={row.session_id ?? row.runtime_id}
      ariaLabel={t.detailsAria}
      closeLabel={t.closeDetails}
      onClose={onClose}
    >
      {row.session_id && onViewActivity && (
        <button
          type="button"
          onClick={() => onViewActivity(row.session_id as string)}
          aria-label={t.viewActivityAria}
          className="self-start font-ui font-semibold text-[12px] border border-line rounded-control px-2.5 py-1.5 text-dim hover:text-fg hover:shadow-glow-amber transition-[color,box-shadow] cursor-pointer inline-flex items-center gap-1.5"
        >
          <Activity aria-hidden="true" className="w-3 h-3" />
          {t.viewActivity}
        </button>
      )}

      <DetailSection title={t.detailStatus}>
        <DetailField
          label={t.colSandboxStatus}
          value={status ? t.sandboxStatus[status] : row.status}
        />
        {/* The backend-native state, preserved verbatim alongside — never
            instead of — the normalized one. */}
        <DetailField label={t.dRawStatus} value={row.raw_status} />
        <DetailField label={t.dStatusReason} value={row.status_reason} />
        <DetailField label={t.dStatusMessage} value={row.status_message} />
        <DetailField label={t.dLastTransition} value={row.last_transition_at} />
        <DetailField label={t.dDeletionAt} value={row.deletion_timestamp} />
      </DetailSection>

      <DetailSection title={t.detailRuntime}>
        <DetailField label={t.fBackend} value={backend ? t.backendKind[backend] : row.backend} />
        <DetailField label={t.dRuntimeId} value={row.runtime_id} copyLabel={t.copyRuntimeId} />
        <DetailField label={t.dRuntimeName} value={row.runtime_name} />
        <DetailField label={t.dRuntimeUid} value={row.runtime_uid} />
        <DetailField label={t.dLocation} value={displayLocation(row.backend_location)} />
        <DetailField label={t.dManaged} value={row.managed ? t.yes : t.no} />
        <DetailField label={t.dMetadataState} value={metadata} />
        <DetailField
          label={t.dRestarts}
          value={row.restart_count === null ? t.notReported : row.restart_count}
        />
      </DetailSection>

      <DetailSection title={t.detailIdentity}>
        <DetailField
          label={t.colCreator}
          value={
            row.creator_login
              ? `@${row.creator_login}`
              : row.creator_id !== null
                ? `#${row.creator_id}`
                : t.unknownCreator
          }
        />
        <DetailField label={t.dActorId} value={row.creator_id} />
        <DetailField
          label={t.dTriggerAuthor}
          value={
            row.trigger_author_login
              ? `@${row.trigger_author_login}`
              : row.trigger_author_id
          }
        />
        <DetailField
          label={t.dAttribution}
          value={attribution ? t.attribution[attribution] : row.attribution_source}
        />
      </DetailSection>

      <DetailSection title={t.detailCorrelation}>
        <DetailField label={t.dSessionId} value={row.session_id} copyLabel={t.copySessionId} />
        <DetailField label={t.dRepo} value={row.repo_full_name} />
        <DetailField label={t.dTriggerIssue} value={row.trigger_issue} />
        <DetailField label={t.dInstallation} value={row.installation_id} />
      </DetailSection>

      <DetailSection title={t.detailLifetime}>
        <DetailField label={t.dCreatedAt} value={row.created_at} />
        <DetailField
          label={t.colLifetime}
          value={
            row.max_lifetime_seconds === null
              ? t.unlimited
              : formatDurationSeconds(row.max_lifetime_seconds)
          }
        />
        <DetailField label={t.dExpiresAt} value={row.expires_at} />
        <DetailField
          label={t.dMinLifetime}
          value={formatDurationSeconds(row.minimum_lifetime_seconds)}
        />
        <DetailField label={t.dIdleGrace} value={formatDurationSeconds(row.idle_grace_seconds)} />
        <DetailField label={t.dLastPending} value={row.last_pending_at ?? t.neverPending} />
      </DetailSection>

      {row.warning_codes.length > 0 && (
        <section className="flex flex-col gap-1.5">
          <span className="font-mono text-eyebrow text-ghost uppercase">
            {t.inventoryWarnings}
          </span>
          <div className="flex items-center gap-1 flex-wrap">
            {row.warning_codes.map((code) => {
              const warning = asMember(SANDBOX_WARNINGS, code);
              return (
                <StatusPill key={code} tone="amber">
                  {warning ? t.warning[warning] : code}
                </StatusPill>
              );
            })}
          </div>
        </section>
      )}
    </DetailsPanel>
  );
}
