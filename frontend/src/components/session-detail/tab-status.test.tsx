import { afterEach, describe, it, expect, vi } from 'vitest';
import type { ReactNode } from 'react';
import { render, screen, within } from '@testing-library/react';
import type { IssueDetail, SessionDetail, SessionRecoveryProjection } from '@/lib/api/types';
import { TabStatus } from './tab-status';

// recharts cannot measure itself under jsdom (it warns width/height 0 and draws
// nothing). The donut's numbers + legend are plain HTML overlays, so passthrough
// stubs keep the tab's assertions deterministic and the output warning-free;
// status-charts.test.tsx covers the props the chart hands to <Pie>.
vi.mock('recharts', () => {
  const Passthrough = ({ children }: { children?: ReactNode }) => <div>{children}</div>;
  return {
    ResponsiveContainer: Passthrough,
    PieChart: Passthrough,
    Pie: ({ children }: { children?: ReactNode }) => <div>{children}</div>,
    Cell: () => null,
    Tooltip: () => null,
  };
});

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
  work_issues: [issue({ number: 9, title: 'do the thing', labels: ['fkst-dev:implementing'] })],
  log_url: null,
  liveness: 'live',
  prs: [],
  ...over,
});

const recovery = (over: Partial<SessionRecoveryProjection> = {}): SessionRecoveryProjection => ({
  state: 'normal',
  reason: 'runtime_live',
  open_work_items: 1,
  runtime: 'live',
  ...over,
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('TabStatus', () => {
  it('renders the decoded phase pill, liveness and a per-work-item state chip', () => {
    render(<TabStatus session={session()} />);
    // "Active" also appears in the lifecycle strip; the pill is the chip.
    expect(screen.getByText('Active', { selector: '.rounded-chip' })).toBeInTheDocument();
    expect(screen.getByText('live')).toBeInTheDocument();
    // The work issue decodes to Implementing.
    expect(screen.getByRole('link', { name: '#9' })).toBeInTheDocument();
    expect(screen.getByText('Implementing')).toBeInTheDocument();
  });

  it('links both the work-item number and its title to the GitHub issue', () => {
    render(<TabStatus session={session()} />);
    const number = screen.getByRole('link', { name: '#9' });
    const title = screen.getByRole('link', { name: 'do the thing' });
    expect(number).toHaveAttribute('href', 'https://github.com/o/r/issues/9');
    expect(title).toHaveAttribute('href', 'https://github.com/o/r/issues/9');
    expect(title).toHaveAttribute('target', '_blank');
  });

  it('shows the empty work-item note when there are none', () => {
    render(<TabStatus session={session({ work_issues: [] })} />);
    // The section note is distinct from the donut's own empty note, so exactly
    // one "No work items yet." shows (getByText throws on a collision).
    expect(screen.getByText('No work items yet.')).toBeInTheDocument();
    // The distribution card shows its own friendly note rather than a hollow ring.
    expect(screen.getByText('No items to chart.')).toBeInTheDocument();
    // The progress meter reads a 0% ratio (no divide-by-zero on zero items).
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '0');
  });

  it('renders the overview grid: progress meter + distribution + lifecycle cards', () => {
    const mixed = session({
      work_issues: [
        issue({ number: 1, state: 'closed' }), // done
        issue({ number: 2, labels: ['fkst-dev:ready'] }), // ready
        issue({ number: 3, labels: ['fkst-dev:implementing'] }), // in progress
        issue({ number: 4, labels: ['fkst-dev:impl-failed'] }), // failed
        issue({ number: 5, labels: ['fkst-dev:enabled'] }), // queued
      ],
    });
    render(<TabStatus session={mixed} />);

    // Both overview chart cards are present and labelled.
    expect(screen.getByLabelText('Progress')).toBeInTheDocument();
    expect(screen.getByLabelText('Distribution')).toBeInTheDocument();

    // 1 of 5 done → 20% meter.
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '20');

    // "In progress" heads both a progress sub-stat and a donut legend row.
    expect(screen.getAllByText('In progress')).toHaveLength(2);
  });

  it('renders bounded recovery diagnostics with count and observed runtime', () => {
    render(
      <TabStatus
        session={session({
          liveness: null,
          recovery: recovery({
            state: 'recovering',
            reason: 'runtime_absent',
            open_work_items: 2,
            runtime: 'absent',
          }),
        })}
      />
    );

    const diagnostics = screen.getByRole('region', { name: 'Recovery' });
    expect(within(diagnostics).getByText('Recovering')).toBeInTheDocument();
    expect(
      within(diagnostics).getByText('Open work is waiting for a runtime.')
    ).toBeInTheDocument();
    expect(within(diagnostics).getByText('Open work')).toBeInTheDocument();
    expect(within(diagnostics).getByText('2')).toBeInTheDocument();
    expect(within(diagnostics).getByText('Absent')).toBeInTheDocument();
  });

  it('issues no live-engine copy or fetch of its own', () => {
    // Status is the lifecycle view (#5841): every live-runtime concern, and the
    // pod exec behind it, belongs to the Engine tab.
    render(<TabStatus session={session()} />);
    expect(screen.queryByText(/Live engine details/)).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Live engine details' })).not.toBeInTheDocument();
  });

  it('keeps useful recovery diagnostics for older responses without a projection', () => {
    render(<TabStatus session={session()} />);
    const diagnostics = screen.getByRole('region', { name: 'Recovery' });
    expect(within(diagnostics).getByText('Normal')).toBeInTheDocument();
    expect(within(diagnostics).getByText('The runtime is live.')).toBeInTheDocument();
    expect(within(diagnostics).getByText('Live')).toBeInTheDocument();
  });

  it('preserves a legacy configuration-rejected reason in fallback diagnostics', () => {
    render(
      <TabStatus
        session={session({
          invalid_reason: 'frozen configuration changed',
          status_labels: ['fkst-config-rejected'],
          liveness: null,
        })}
      />
    );

    const diagnostics = screen.getByRole('region', { name: 'Recovery' });
    expect(
      within(diagnostics).getByText('A frozen configuration change was rejected.')
    ).toBeInTheDocument();
  });

  it('renders the session timeline (started → now) in the Status tab', () => {
    render(<TabStatus session={session()} />);
    expect(screen.getByText('Timeline')).toBeInTheDocument();
    expect(screen.getByText('Session started')).toBeInTheDocument();
  });
});

/// Layout contract (#5842). The timeline narrates what happened to the very work
/// items listed beside it; stacked, the list started below the fold of the thing
/// that refers to it. Both panes now share one grid and each scrolls its own
/// content, so neither can scroll the other away.
describe('TabStatus layout', () => {
  it('lays the timeline and the work items side by side in one grid', () => {
    const { container } = render(<TabStatus session={session()} />);
    const timeline = screen.getByRole('region', { name: 'Timeline' });
    const workItems = screen.getByRole('region', { name: 'Work items' });

    const grid = container.querySelector(
      '.grid.md\\:grid-cols-\\[var\\(--split-start\\)_minmax\\(0\\,1fr\\)\\]'
    );
    expect(grid, 'the split must use the shared SplitPanes grid').not.toBeNull();
    expect(grid!.contains(timeline)).toBe(true);
    expect(grid!.contains(workItems)).toBe(true);
    // Peer panes, not a rail: the first track is an equal fraction.
    expect(grid!.getAttribute('style')).toContain('minmax(0,1fr)');
    // Timeline first, work items second.
    expect(grid!.firstElementChild).toBe(timeline);
  });

  it('shrinks below content so each pane scrolls instead of growing the row', () => {
    // A grid item's min-height defaults to its content height, so without
    // `min-h-0` the row would grow past the panel and the panes' own scrollers
    // would never engage — the whole tab would scroll instead.
    render(<TabStatus session={session()} />);
    expect(screen.getByRole('region', { name: 'Timeline' })).toHaveClass('min-h-0');
    expect(screen.getByRole('region', { name: 'Work items' })).toHaveClass('min-h-0');
  });

  it('gives each pane its own scroll container', () => {
    render(
      <TabStatus
        session={session({
          work_issues: [issue({ number: 9 }), issue({ number: 10 }), issue({ number: 11 })],
        })}
      />
    );
    const timelineScroller = screen
      .getByRole('region', { name: 'Timeline' })
      .querySelector('.overflow-y-auto');
    const workScroller = screen
      .getByRole('region', { name: 'Work items' })
      .querySelector('.overflow-y-auto');
    expect(timelineScroller).not.toBeNull();
    expect(workScroller).not.toBeNull();
    expect(timelineScroller).not.toBe(workScroller);
  });

  it('keeps the overview band out of the split so it is never squeezed', () => {
    const { container } = render(<TabStatus session={session()} />);
    const overview = container.querySelector('section.grid.flex-none');
    expect(overview, 'the overview band must not be a flex-1 sibling').not.toBeNull();
    expect(screen.getByRole('region', { name: 'Progress' }).closest('.grid')).toBe(overview);
    // …and it is not inside the split.
    expect(overview!.contains(screen.getByRole('region', { name: 'Timeline' }))).toBe(false);
  });

  it('still renders the empty work-item note inside the pane', () => {
    render(<TabStatus session={session({ work_issues: [] })} />);
    const pane = screen.getByRole('region', { name: 'Work items' });
    expect(within(pane).getByText('No work items yet.')).toBeInTheDocument();
  });
});
