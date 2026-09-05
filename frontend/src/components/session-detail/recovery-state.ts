import { decodeSessionStatus, isRetiredWorkItem } from '@/lib/api/derive';
import type { SessionDetail, SessionRecoveryProjection } from '@/lib/api/types';

// The two session-level rules the Status and Engine tabs both depend on. Pure
// and unit-tested, because one of them decides whether a pod exec is permitted.

/**
 * Whether the session's RUNTIME is positively live.
 *
 * The live-engine observe read pod-execs into the running pod, so it is only
 * meaningful — and only permitted — while the runtime is live. The typed
 * projection wins whenever it is present, so a stale legacy `liveness: 'live'`
 * cannot re-enable a pod exec after an authoritative absent/terminal
 * observation.
 */
export function isRuntimeLive(session: SessionDetail): boolean {
  return session.recovery ? session.recovery.runtime === 'live' : session.liveness === 'live';
}

/**
 * Derive a recovery projection for a session the API did not send one for, so
 * the operator read model degrades to the decoded lifecycle rather than
 * disappearing.
 */
export function fallbackRecovery(session: SessionDetail): SessionRecoveryProjection {
  const openWork = session.work_issues.filter(
    (issue) => issue.state === 'open' && !isRetiredWorkItem(issue)
  ).length;
  const status = decodeSessionStatus(session);
  const runtime = session.liveness ?? 'unknown';

  switch (status.phase) {
    case 'invalid':
      return {
        state: 'invalid',
        reason: session.status_labels.includes('fkst-config-rejected')
          ? 'configuration_rejected'
          : 'registration_invalid',
        open_work_items: 0,
        runtime,
      };
    case 'retired':
      return { state: 'retired', reason: 'trigger_closed', open_work_items: 0, runtime };
    case 'degraded':
      return {
        state: 'degraded',
        reason: 'runtime_health_degraded',
        open_work_items: openWork,
        runtime,
      };
    case 'idle':
      return { state: 'idle', reason: 'no_pending_work', open_work_items: 0, runtime };
    case 'active':
      return { state: 'normal', reason: 'runtime_live', open_work_items: openWork, runtime };
    default:
      if (openWork > 0 && session.liveness === 'starting') {
        return {
          state: 'recovering',
          reason: 'runtime_starting',
          open_work_items: openWork,
          runtime,
        };
      }
      if (openWork > 0 && session.liveness === 'terminating') {
        return {
          state: 'recovering',
          reason: 'runtime_terminating',
          open_work_items: openWork,
          runtime,
        };
      }
      return {
        state: 'unknown',
        reason: 'runtime_observation_unavailable',
        open_work_items: openWork,
        runtime,
      };
  }
}
