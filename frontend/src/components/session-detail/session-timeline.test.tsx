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
  creator: 'shining',
  work_label: 'fkst-work',
  auto_merge: true,
  environment: null,
  source_branch: null,
  target_branch: 'fkst-hosted-default',
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

  it('excludes retired and partial-readmission work events', () => {
    const nodes = buildTimeline(
      session({
        work_issues: [
          issue({ number: 10, title: 'retired', labels: ['fkst-session-retired'] }),
          issue({
            number: 11,
            title: 'partial readmission',
            labels: ['fkst-session-retired', 'fkst-picked-up'],
          }),
          issue({ number: 12, title: 'active' }),
        ],
      })
    );

    expect(nodes.map((node) => node.key)).toEqual(['started', 'work-12-queued', 'now']);
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
    // Work / PR references link out to GitHub (issue vs PR URL by node kind).
    // Issue #11 appears twice: once on its queued node, once on its finished node.
    const workRefs = screen.getAllByRole('link', { name: /#11/ });
    expect(workRefs).toHaveLength(2);
    for (const ref of workRefs) {
      expect(ref).toHaveAttribute('href', 'https://github.com/o/r/issues/11');
    }
    const prRef = screen.getByRole('link', { name: /#20/ });
    expect(prRef).toHaveAttribute('href', 'https://github.com/o/r/pull/20');
    expect(prRef).toHaveAttribute('target', '_blank');
    // Terminal "now" node names the derived state (live → active).
    expect(
      screen.getByText((text) => text.startsWith('Now') && text.includes('Active'))
    ).toBeInTheDocument();
    // Timestamps are rendered in SGT (Asia/Singapore) with the zone suffix.
    expect(screen.getAllByText(/SGT/).length).toBeGreaterThan(0);
  });

  it('does not render retired or partial-readmission work references', () => {
    render(
      <SessionTimeline
        session={session({
          work_issues: [
            issue({ number: 10, title: 'retired', labels: ['fkst-session-retired'] }),
            issue({
              number: 11,
              title: 'partial readmission',
              labels: ['fkst-session-retired', 'fkst-picked-up'],
            }),
            issue({ number: 12, title: 'active' }),
          ],
        })}
      />
    );

    expect(screen.queryByRole('link', { name: /#10/ })).not.toBeInTheDocument();
    expect(screen.queryByRole('link', { name: /#11/ })).not.toBeInTheDocument();
    expect(screen.getByRole('link', { name: /#12/ })).toBeInTheDocument();
  });

  it('is a pane the caller can size, and scrolls its own history', () => {
    // Laid beside the work items (#5842), a long history must overflow INSIDE
    // this pane rather than growing the grid row and pushing the list out of
    // view. The className goes straight onto the card because an extra wrapper
    // would become the grid item and defeat the min-h-0 chain.
    render(<SessionTimeline session={session()} className="min-h-0" />);
    const card = screen.getByRole('region', { name: 'Timeline' });
    expect(card).toHaveClass('min-h-0');
    const list = screen.getByRole('list');
    expect(list.closest('.overflow-y-auto')).not.toBeNull();
    expect(card.contains(list)).toBe(true);
  });

  it('names the paused state on the now node for an idle session', () => {
    render(
      <SessionTimeline
        session={session({
          liveness: null,
          status_labels: ['fkst-substrate-active'],
          work_issues: [],
        })}
      />
    );
    expect(
      screen.getByText((text) => text.startsWith('Now') && text.includes('Idle'))
    ).toBeInTheDocument();
  });
});
