import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import type { IssueDetail, PrDetail, SessionDetail } from '@/lib/api/types';
import { buildTimeline, SessionTimeline } from './session-timeline';

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

const pr = (over: Partial<PrDetail> & Pick<PrDetail, 'number'>): PrDetail => ({
  title: `pr ${over.number}`,
  html_url: `https://github.com/o/r/pull/${over.number}`,
  state: 'open',
  merged: false,
  work_issue: null,
  ...over,
});

const session = (over: Partial<SessionDetail> = {}): SessionDetail => ({
  session_id: 'sess-1',
  name: 'nightly',
  work_label: 'fkst-work',
  auto_merge: true,
  environment: null,
  packages: [],
  invalid_reason: null,
  status_labels: ['fkst-substrate-active'],
  trigger: issue({ number: 7, created_at: '2026-07-01T00:00:00Z' }),
  work_issues: [],
  log_url: null,
  liveness: 'live',
  prs: [],
  ...over,
});

describe('buildTimeline', () => {
  it('orders events chronologically: started → work (interleaved) → PRs → now', () => {
    const nodes = buildTimeline(
      session({
        trigger: issue({ number: 7, created_at: '2026-07-01T00:00:00Z' }),
        work_issues: [
          issue({ number: 10, state: 'open', created_at: '2026-07-02T00:00:00Z' }),
          issue({
            number: 11,
            state: 'closed',
            created_at: '2026-07-01T12:00:00Z',
            closed_at: '2026-07-03T00:00:00Z',
          }),
        ],
        prs: [pr({ number: 20, merged: true, state: 'closed' }), pr({ number: 21, state: 'open' })],
      })
    );

    // Sorted by real timestamp; the untimed PR nodes park before the terminal
    // "now" node.
    expect(nodes.map((n) => n.kind)).toEqual([
      'started',
      'work-queued', // #11 queued (Jul 1 12:00) sorts before #10 (Jul 2)
      'work-queued', // #10 queued
      'work-finished', // #11 finished (Jul 3)
      'pr-merged', // #20
      'pr-opened', // #21
      'now',
    ]);
    expect(nodes.map((n) => n.key)).toEqual([
      'started',
      'work-11-queued',
      'work-10-queued',
      'work-11-finished',
      'pr-20',
      'pr-21',
      'now',
    ]);
    // The terminal node carries the derived phase (live pod → active).
    expect(nodes.at(-1)).toMatchObject({ kind: 'now', phase: 'active', tone: 'live' });
  });

  it('always begins with a started node and ends with the now node', () => {
    const nodes = buildTimeline(session({ work_issues: [], prs: [] }));
    expect(nodes.map((n) => n.kind)).toEqual(['started', 'now']);
  });

  it('reflects the derived phase on the now node — idle when paused', () => {
    // Announced, no live pod, no open work → idle (paused).
    const nodes = buildTimeline(
      session({ liveness: null, status_labels: ['fkst-substrate-active'], work_issues: [] })
    );
    expect(nodes.at(-1)).toMatchObject({ kind: 'now', phase: 'idle' });
  });
});

describe('SessionTimeline', () => {
  it('renders the labelled rail with SGT timestamps and the current state', () => {
    render(
      <SessionTimeline
        session={session({
          work_issues: [
            issue({
              number: 11,
              state: 'closed',
              created_at: '2026-07-01T12:00:00Z',
              closed_at: '2026-07-03T00:00:00Z',
            }),
          ],
          prs: [pr({ number: 20, merged: true, state: 'closed' })],
        })}
      />
    );

    expect(screen.getByText('Timeline')).toBeInTheDocument();
    expect(screen.getByText('Session started')).toBeInTheDocument();
    expect(screen.getByText('Work item queued')).toBeInTheDocument();
    expect(screen.getByText('Work item finished')).toBeInTheDocument();
    expect(screen.getByText('Pull request merged')).toBeInTheDocument();
    // Work / PR references render as plain (non-link) text.
    expect(screen.getByText('#20')).toBeInTheDocument();
    // Terminal "now" node names the derived state (live → active).
    expect(
      screen.getByText((text) => text.startsWith('Now') && text.includes('Active'))
    ).toBeInTheDocument();
    // Timestamps are rendered in SGT (Asia/Singapore) with the zone suffix.
    expect(screen.getAllByText(/SGT/).length).toBeGreaterThan(0);
  });

  it('names the paused state on the now node for an idle session', () => {
    render(
      <SessionTimeline
        session={session({ liveness: null, status_labels: ['fkst-substrate-active'], work_issues: [] })}
      />
    );
    expect(
      screen.getByText((text) => text.startsWith('Now') && text.includes('Idle'))
    ).toBeInTheDocument();
  });
});
