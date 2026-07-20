import { describe, it, expect, beforeEach, vi } from 'vitest';
import type { ReactNode } from 'react';
import { render, screen } from '@testing-library/react';
import type { IssueDetail } from '@/lib/api/types';

// ---- Mocks ------------------------------------------------------------------
// reduced-motion is a mutable knob so one suite exercises both the animated and
// the instant-render branches without remounting the module.
const motionState = { reduced: false };
vi.mock('framer-motion', () => ({ useReducedMotion: () => motionState.reduced }));

// recharts cannot measure itself under jsdom, and this suite only cares about
// the data + animation props the donut hands to <Pie> (its numbers/legend are
// plain HTML overlays). So we replace recharts with thin capture stubs.
const pieProps: Record<string, unknown>[] = [];
vi.mock('recharts', () => {
  const Passthrough = ({ children }: { children?: ReactNode }) => <div>{children}</div>;
  return {
    ResponsiveContainer: Passthrough,
    PieChart: Passthrough,
    Tooltip: () => null,
    Cell: () => null,
    Pie: ({ children, ...rest }: { children?: ReactNode }) => {
      pieProps.push(rest);
      return <div data-testid="pie">{children}</div>;
    },
  };
});

import { ProgressCard, WorkDonut, countWorkItems } from './status-charts';

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

/** A work list spanning every group: 1 done (closed), 1 ready, 3 in-progress
 *  (implementing + thinking + claimed), 1 failed, 2 queued (enabled + unlabeled). */
const MIXED: IssueDetail[] = [
  issue({ number: 1, state: 'closed' }),
  issue({ number: 2, labels: ['fkst-dev:ready'] }),
  issue({ number: 3, labels: ['fkst-dev:implementing'] }),
  issue({ number: 4, labels: ['fkst-dev:thinking'] }),
  issue({ number: 5, labels: ['fkst-dev:claimed'] }),
  issue({ number: 6, labels: ['fkst-dev:impl-failed'] }),
  issue({ number: 7, labels: ['fkst-dev:enabled'] }),
  issue({ number: 8, labels: [] }),
];

beforeEach(() => {
  pieProps.length = 0;
  motionState.reduced = false;
});

describe('countWorkItems (pure grouping)', () => {
  it('folds every decoded state into the five overview groups', () => {
    expect(countWorkItems(MIXED)).toEqual({
      total: 8,
      done: 1,
      ready: 1,
      inProgress: 3, // implementing + thinking + claimed
      failed: 1,
      queued: 2, // enabled + unlabeled
    });
  });

  it('returns an all-zero shape for an empty list', () => {
    expect(countWorkItems([])).toEqual({
      total: 0,
      done: 0,
      ready: 0,
      inProgress: 0,
      failed: 0,
      queued: 0,
    });
  });
});

describe('ProgressCard', () => {
  it('shows the done/total headline, a percent meter, and the sub-counts', () => {
    render(<ProgressCard counts={countWorkItems(MIXED)} />);

    expect(screen.getByLabelText('Progress')).toBeInTheDocument();
    // 1 of 8 done → round(12.5) = 13%.
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '13');
    // The sub-stats surface the in-progress / ready / failed counts.
    expect(screen.getByText('In progress')).toBeInTheDocument();
    expect(screen.getByText('Ready')).toBeInTheDocument();
    expect(screen.getByText('Failed')).toBeInTheDocument();
  });

  it('handles zero work items: 0% meter and no sub-counts', () => {
    render(<ProgressCard counts={countWorkItems([])} />);
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '0');
    // No breakdown to show when there is nothing yet.
    expect(screen.queryByText('In progress')).not.toBeInTheDocument();
  });
});

describe('WorkDonut', () => {
  it('feeds only the non-empty groups to the ring and centers the total', () => {
    render(<WorkDonut counts={countWorkItems(MIXED)} />);

    expect(screen.getByLabelText('Distribution')).toBeInTheDocument();
    // All five groups are non-zero in MIXED, so all five slices are charted.
    expect(pieProps).toHaveLength(1);
    expect((pieProps[0]!.data as unknown[]).length).toBe(5);
    // The centered total equals the work-item count.
    expect(screen.getByText('8')).toBeInTheDocument();
    // The legend labels every charted group.
    expect(screen.getByText('Done')).toBeInTheDocument();
    expect(screen.getByText('Queued')).toBeInTheDocument();
  });

  it('drops zero-count groups from the ring', () => {
    // Only closed (done) + one failed → two non-zero slices.
    const counts = countWorkItems([
      issue({ number: 1, state: 'closed' }),
      issue({ number: 2, labels: ['fkst-dev:impl-failed'] }),
    ]);
    render(<WorkDonut counts={counts} />);
    expect((pieProps[0]!.data as unknown[]).length).toBe(2);
  });

  it('enables the ring sweep when motion is allowed', () => {
    render(<WorkDonut counts={countWorkItems(MIXED)} />);
    expect(pieProps[0]!.isAnimationActive).toBe(true);
  });

  it('renders instantly (no animation) under reduced motion', () => {
    motionState.reduced = true;
    render(<WorkDonut counts={countWorkItems(MIXED)} />);
    expect(pieProps[0]!.isAnimationActive).toBe(false);
  });

  it('shows a friendly note and no ring when there are no work items', () => {
    render(<WorkDonut counts={countWorkItems([])} />);
    expect(screen.getByText('No items to chart.')).toBeInTheDocument();
    expect(screen.queryByTestId('pie')).not.toBeInTheDocument();
  });
});
