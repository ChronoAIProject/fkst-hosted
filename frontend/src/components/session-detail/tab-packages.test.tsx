import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import type { IssueDetail, SessionDetail } from '@/lib/api/types';
import { TabPackages } from './tab-packages';
import type { ObserveState } from './observe-state';

const trigger: IssueDetail = {
  number: 7,
  title: 'sess',
  state: 'open',
  author: 'shining',
  labels: [],
  html_url: 'https://github.com/o/r/issues/7',
  created_at: '2026-07-01T00:00:00Z',
  updated_at: '2026-07-01T00:00:00Z',
  closed_at: null,
};

const session = (over: Partial<SessionDetail> = {}): SessionDetail => ({
  session_id: 'sess-1',
  name: 'nightly',
  work_label: null,
  auto_merge: null,
  environment: null,
  packages: ['ChronoAIProject/fkst-packages@fkst-hosted:codex/triage'],
  invalid_reason: null,
  status_labels: [],
  trigger,
  work_issues: [],
  log_url: null,
  liveness: null,
  prs: [],
  ...over,
});

const idle: ObserveState = { status: 'idle' };

describe('TabPackages', () => {
  it('renders each package as a role + short handle + full ref', () => {
    render(<TabPackages session={session()} observe={idle} />);
    expect(screen.getByText('Triage')).toBeInTheDocument();
    expect(screen.getByText('triage')).toBeInTheDocument();
    expect(
      screen.getByText('ChronoAIProject/fkst-packages@fkst-hosted:codex/triage')
    ).toBeInTheDocument();
  });

  it('shows the empty note when the session declares no packages', () => {
    render(<TabPackages session={session({ packages: [] })} observe={idle} />);
    expect(screen.getByText('This session declares no packages.')).toBeInTheDocument();
  });

  it('surfaces queue activity only once observe has loaded', () => {
    const { rerender } = render(<TabPackages session={session()} observe={idle} />);
    expect(screen.queryByText('Queue activity')).not.toBeInTheDocument();

    rerender(
      <TabPackages
        session={session()}
        observe={{ status: 'loaded', snapshot: { queues: [{ queue: 'events', depth: 5 }] } }}
      />
    );
    expect(screen.getByText('Queue activity')).toBeInTheDocument();
    expect(screen.getByText('events')).toBeInTheDocument();
  });
});
