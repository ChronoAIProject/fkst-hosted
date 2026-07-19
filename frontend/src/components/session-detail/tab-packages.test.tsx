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
  log_access: null,
  output_lang: null,
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

  it('renders a copyable ref button on each package row', () => {
    render(
      <TabPackages
        session={session({
          packages: [
            'ChronoAIProject/fkst-packages@fkst-hosted:codex/triage',
            'ChronoAIProject/fkst-packages@fkst-hosted:tools/lint',
          ],
        })}
        observe={idle}
      />
    );
    // One copy affordance per declared package, all labelled the same.
    expect(screen.getAllByRole('button', { name: 'Copy ref' })).toHaveLength(2);
  });

  it('renders the frozen configuration with all fields populated', () => {
    render(
      <TabPackages
        session={session({
          work_label: 'nightly-work',
          environment: 'video-studio',
          auto_merge: true,
          output_lang: 'zh',
          log_access: ['alice', 'bob'],
        })}
        observe={idle}
      />
    );
    expect(screen.getByText('Configuration')).toBeInTheDocument();
    expect(screen.getByText('Frozen at registration — these cannot be changed.')).toBeInTheDocument();

    // Scalars.
    expect(screen.getByText('Work label')).toBeInTheDocument();
    expect(screen.getByText('nightly-work')).toBeInTheDocument();
    expect(screen.getByText('video-studio')).toBeInTheDocument();
    expect(screen.getByText('zh')).toBeInTheDocument();
    // auto_merge = true -> "Yes".
    expect(screen.getByText('Yes')).toBeInTheDocument();

    // Log-access allowlist rendered as one chip per viewer.
    expect(screen.getByText('Log access')).toBeInTheDocument();
    expect(screen.getByText('alice')).toBeInTheDocument();
    expect(screen.getByText('bob')).toBeInTheDocument();
  });

  it('renders "No" when auto-merge is disabled', () => {
    render(<TabPackages session={session({ auto_merge: false })} observe={idle} />);
    expect(screen.getByText('No')).toBeInTheDocument();
  });

  it('renders an explicit "None" for an empty log-access allowlist', () => {
    // Empty list, null, and undefined must all read as "no additional viewers"
    // rather than a blank cell — an unset allowlist is a real, frozen state.
    for (const log_access of [[] as string[], null, undefined]) {
      const { unmount } = render(
        <TabPackages session={session({ log_access })} observe={idle} />
      );
      expect(screen.getByText('Log access')).toBeInTheDocument();
      expect(screen.getByText('None')).toBeInTheDocument();
      unmount();
    }
  });
});
