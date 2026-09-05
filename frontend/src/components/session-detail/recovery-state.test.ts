import { describe, it, expect } from 'vitest';
import type { IssueDetail, SessionDetail } from '@/lib/api/types';
import { fallbackRecovery, isRuntimeLive } from './recovery-state';

const issue = (over: Partial<IssueDetail> & Pick<IssueDetail, 'number'>): IssueDetail => ({
  title: `issue ${over.number}`,
  state: 'open',
  author: 'shining',
  labels: [],
  html_url: `https://github.com/o/r/issues/${over.number}`,
  created_at: '2026-07-01T00:00:00Z',
  updated_at: '2026-07-01T00:00:00Z',
  closed_at: null,
  ...over,
});

const session = (over: Partial<SessionDetail> = {}): SessionDetail => ({
  session_id: 'sess-1',
  name: 'nightly',
  creator: 'shining',
  work_label: 'fkst-work',
  auto_merge: true,
  environment: null,
  source_branch: null,
  target_branch: 'fkst-hosted-default',
  packages: [],
  invalid_reason: null,
  status_labels: ['fkst-substrate-active'],
  trigger: issue({ number: 7 }),
  work_issues: [issue({ number: 9 })],
  log_url: null,
  liveness: 'live',
  prs: [],
  ...over,
});

describe('isRuntimeLive', () => {
  it('is true only when the typed projection reports a live runtime', () => {
    expect(
      isRuntimeLive(
        session({
          recovery: {
            state: 'normal',
            reason: 'runtime_live',
            open_work_items: 1,
            runtime: 'live',
          },
        })
      )
    ).toBe(true);
  });

  it('lets the projection overrule a stale legacy liveness', () => {
    // The gate gates a POD EXEC. A stale `liveness: 'live'` must never re-enable
    // it after an authoritative absent observation.
    expect(
      isRuntimeLive(
        session({
          liveness: 'live',
          recovery: {
            state: 'retired',
            reason: 'trigger_closed',
            open_work_items: 0,
            runtime: 'absent',
          },
        })
      )
    ).toBe(false);
  });

  it('falls back to legacy liveness when no projection is present', () => {
    expect(isRuntimeLive(session({ liveness: 'live' }))).toBe(true);
    expect(isRuntimeLive(session({ liveness: 'starting' }))).toBe(false);
    expect(isRuntimeLive(session({ liveness: null }))).toBe(false);
  });
});

describe('fallbackRecovery', () => {
  it('separates a rejected configuration from an invalid registration', () => {
    expect(
      fallbackRecovery(session({ invalid_reason: 'bad', status_labels: ['fkst-config-rejected'] }))
    ).toMatchObject({ state: 'invalid', reason: 'configuration_rejected', open_work_items: 0 });

    expect(
      fallbackRecovery(
        session({ invalid_reason: 'bad', status_labels: ['fkst-substrate-invalid'] })
      )
    ).toMatchObject({ state: 'invalid', reason: 'registration_invalid' });
  });

  it('reports a retired session as trigger-closed with no open work', () => {
    expect(
      fallbackRecovery(
        session({
          status_labels: ['fkst-session-retired'],
          trigger: issue({ number: 7, state: 'closed' }),
        })
      )
    ).toMatchObject({ state: 'retired', reason: 'trigger_closed', open_work_items: 0 });
  });

  it('reports a degraded session and keeps its open work count', () => {
    expect(
      fallbackRecovery(
        session({
          status_labels: ['fkst-substrate-active', 'fkst-degraded'],
          work_issues: [issue({ number: 9 }), issue({ number: 10 })],
        })
      )
    ).toMatchObject({ state: 'degraded', reason: 'runtime_health_degraded', open_work_items: 2 });
  });

  it('excludes retired and partial-readmission work from fallback counts', () => {
    expect(
      fallbackRecovery(
        session({
          liveness: 'starting',
          status_labels: [],
          work_issues: [
            issue({ number: 9, labels: ['fkst-session-retired'] }),
            issue({ number: 10, labels: ['fkst-session-retired', 'fkst-picked-up'] }),
          ],
        })
      )
    ).toMatchObject({
      state: 'unknown',
      reason: 'runtime_observation_unavailable',
      open_work_items: 0,
    });
  });

  it('reports an idle session as having no pending work', () => {
    expect(
      fallbackRecovery(
        session({ work_issues: [issue({ number: 9, state: 'closed' })], liveness: null })
      )
    ).toMatchObject({
      state: 'idle',
      reason: 'no_pending_work',
      open_work_items: 0,
      // A null liveness degrades to the 'unknown' runtime label rather than
      // asserting anything about a pod nobody observed.
      runtime: 'unknown',
    });
  });

  it('reports an active session as normal and live', () => {
    expect(fallbackRecovery(session())).toMatchObject({
      state: 'normal',
      reason: 'runtime_live',
      open_work_items: 1,
      runtime: 'live',
    });
  });

  it('reports open work on a starting or terminating runtime as recovering', () => {
    // Neither phase decodes to active/idle, so without these arms an operator
    // would see "unknown" while the pod is demonstrably coming up or going down.
    expect(fallbackRecovery(session({ status_labels: [], liveness: 'starting' }))).toMatchObject({
      state: 'recovering',
      reason: 'runtime_starting',
      open_work_items: 1,
    });

    expect(fallbackRecovery(session({ status_labels: [], liveness: 'terminating' }))).toMatchObject(
      { state: 'recovering', reason: 'runtime_terminating', open_work_items: 1 }
    );
  });

  it('falls through to unknown when nothing can be observed', () => {
    expect(
      fallbackRecovery(session({ status_labels: [], liveness: null, work_issues: [] }))
    ).toMatchObject({
      state: 'unknown',
      reason: 'runtime_observation_unavailable',
      open_work_items: 0,
      runtime: 'unknown',
    });
  });

  it('does not claim recovering for a starting runtime with no open work', () => {
    // The recovering arms are gated on open work: a starting pod with nothing to
    // do is not recovering from anything.
    expect(
      fallbackRecovery(session({ status_labels: [], liveness: 'starting', work_issues: [] }))
    ).toMatchObject({ state: 'unknown', reason: 'runtime_observation_unavailable' });
  });
});
