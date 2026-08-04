import { useContent } from '@/i18n';
import type { ActivityRow } from '@/lib/api/operations';
import {
  ACTOR_KINDS,
  DELIVERY_STATES,
  LIFECYCLE_ACTIONS,
  PRINCIPAL_KINDS,
  asMember,
} from '@/lib/api/operations';
import { argumentEntries } from '@/lib/operations/format';
import { DetailField, DetailSection, DetailsPanel } from './details-panel';

/**
 * The activity row details.
 *
 * Everything shown here is already on the row: the panel adds no second request
 * and no second source. It exposes the fields the table had to abbreviate — the
 * immutable actor id behind the login snapshot, the exact UTC instants behind
 * the local times, the full correlation set, and the safe arguments as DISCRETE
 * fields rather than the table's concatenated summary.
 *
 * There is deliberately no raw URL, no header, no body and no error message
 * anywhere in it: the write side never recorded any of those, and this surface
 * must not become the place someone tries to add them.
 */
export function ActivityDetails({ row, onClose }: { row: ActivityRow; onClose: () => void }) {
  const t = useContent().operations;
  const actorKind = asMember(ACTOR_KINDS, row.actor.kind);
  const principalKind = asMember(PRINCIPAL_KINDS, row.principal.kind);
  const delivery = asMember(DELIVERY_STATES, row.delivery_state);
  const title =
    row.record_kind === 'api_request'
      ? row.operation_id
      : (asMember(LIFECYCLE_ACTIONS, row.lifecycle_action)
          ? t.lifecycleAction[asMember(LIFECYCLE_ACTIONS, row.lifecycle_action)!]
          : row.lifecycle_action);

  return (
    <DetailsPanel
      title={title}
      ariaLabel={t.detailsAria}
      closeLabel={t.closeDetails}
      onClose={onClose}
    >
      <DetailSection title={t.detailIdentity}>
        <DetailField label={t.dActorKind} value={actorKind ? t.actorKind[actorKind] : row.actor.kind} />
        <DetailField label={t.dActorLogin} value={row.actor.login ? `@${row.actor.login}` : null} />
        {/* The immutable id — the only ownership proof — always available here
            even though the table shows the mutable login snapshot. */}
        <DetailField label={t.dActorId} value={row.actor.id} />
        <DetailField
          label={t.dPrincipal}
          value={principalKind ? t.principalKind[principalKind] : row.principal.kind}
        />
      </DetailSection>

      {row.record_kind === 'api_request' ? (
        <DetailSection title={t.detailStatus}>
          <DetailField label={t.dOperation} value={row.operation_id} />
          {/* The normalized template. There is no raw URI to show. */}
          <DetailField label={t.dRoute} value={`${row.method} ${row.route_template}`} />
          <DetailField label={t.dStartedAt} value={row.started_at} />
          <DetailField label={t.dCompletedAt} value={row.completed_at} />
          <DetailField label={t.dErrorCode} value={row.error_code} />
        </DetailSection>
      ) : (
        <DetailSection title={t.detailRuntime}>
          <DetailField label={t.dOccurredAt} value={row.occurred_at} />
          <DetailField label={t.dRuntimeId} value={row.runtime_id} copyLabel={t.copyRuntimeId} />
          <DetailField label={t.dCreatedAt} value={row.created_at} />
          <DetailField label={t.dReasonCode} value={row.reason_code} />
          <DetailField
            label={t.dTriggerAuthor}
            value={row.trigger_author.login ? `@${row.trigger_author.login}` : row.trigger_author.id}
          />
        </DetailSection>
      )}

      <DetailSection title={t.detailCorrelation}>
        <DetailField
          label={t.dSessionId}
          value={
            row.record_kind === 'sandbox_lifecycle' ? row.session_id : row.correlation.session_id
          }
          copyLabel={t.copySessionId}
        />
        <DetailField label={t.dRepo} value={row.correlation.repo_full_name} />
        <DetailField label={t.dTriggerIssue} value={row.correlation.trigger_issue} />
        <DetailField label={t.dInstallation} value={row.correlation.installation_id} />
        <DetailField
          label={t.dRequestId}
          value={
            row.record_kind === 'api_request'
              ? (row.request_id ?? row.correlation.request_id)
              : row.correlation.request_id
          }
          copyLabel={t.copyRequestId}
        />
        <DetailField label={t.dWebhookDelivery} value={row.correlation.webhook_delivery_id} />
        <DetailField label={t.dEventId} value={row.event_id} copyLabel={t.copyEventId} />
      </DetailSection>

      {row.record_kind === 'api_request' && (
        <DetailSection title={t.detailArguments}>
          {argumentEntries(row.arguments).length === 0 ? (
            <DetailField
              label={t.detailArguments}
              value={
                row.arguments_parse_status &&
                row.arguments_parse_status in t.argumentsParseStatus
                  ? t.argumentsParseStatus[
                      row.arguments_parse_status as keyof typeof t.argumentsParseStatus
                    ]
                  : t.noArguments
              }
            />
          ) : (
            argumentEntries(row.arguments).map((entry) => (
              <DetailField key={entry.key} label={entry.key} value={entry.value} />
            ))
          )}
        </DetailSection>
      )}

      <DetailSection title={t.detailDelivery}>
        <DetailField
          label={t.dDeliveryState}
          value={delivery ? t.delivery[delivery] : row.delivery_state}
        />
        <DetailField label={t.dSource} value={row.source} />
      </DetailSection>
    </DetailsPanel>
  );
}
