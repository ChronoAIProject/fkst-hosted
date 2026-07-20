import { afterEach, describe, it, expect, vi } from 'vitest';
import type { ReactNode } from 'react';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { IssueDetail, SessionDetail } from '@/lib/api/types';
import { TabStatus } from './tab-status';
import type { ObserveState } from './observe-state';

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
  work_label: 'fkst-work',
  auto_merge: true,
  environment: null,
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

const idle: ObserveState = { status: 'idle' };

afterEach(() => {
  vi.restoreAllMocks();
});

describe('TabStatus', () => {
  it('renders the decoded phase pill, liveness and a per-work-item state chip', () => {
    render(<TabStatus session={session()} observe={idle} onLoadObserve={() => {}} />);
    // "Active" also appears in the lifecycle strip; the pill is the chip.
    expect(screen.getByText('Active', { selector: '.rounded-chip' })).toBeInTheDocument();
    expect(screen.getByText('live')).toBeInTheDocument();
    // The work issue decodes to Implementing.
    expect(screen.getByRole('link', { name: '#9' })).toBeInTheDocument();
    expect(screen.getByText('Implementing')).toBeInTheDocument();
  });

  it('shows the empty work-item note when there are none', () => {
    render(
      <TabStatus session={session({ work_issues: [] })} observe={idle} onLoadObserve={() => {}} />
    );
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
    render(<TabStatus session={mixed} observe={idle} onLoadObserve={() => {}} />);

    // Both overview chart cards are present and labelled.
    expect(screen.getByLabelText('Progress')).toBeInTheDocument();
    expect(screen.getByLabelText('Distribution')).toBeInTheDocument();

    // 1 of 5 done → 20% meter.
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '20');

    // "In progress" heads both a progress sub-stat and a donut legend row.
    expect(screen.getAllByText('In progress')).toHaveLength(2);
  });

  it('offers the Live engine details button and fires the callback', async () => {
    const user = userEvent.setup();
    const onLoad = vi.fn();
    render(<TabStatus session={session()} observe={idle} onLoadObserve={onLoad} />);
    await user.click(screen.getByRole('button', { name: 'Live engine details' }));
    expect(onLoad).toHaveBeenCalledTimes(1);
  });

  it('renders the session timeline (started → now) in the Status tab', () => {
    render(<TabStatus session={session()} observe={idle} onLoadObserve={() => {}} />);
    expect(screen.getByText('Timeline')).toBeInTheDocument();
    expect(screen.getByText('Session started')).toBeInTheDocument();
  });

  it('gates the live engine on a live pod: paused note, no fetch button when idle', () => {
    // A latched active label whose pod was reaped (no live liveness) with no open
    // work is idle/paused — the observe fetch must NOT be offered.
    const paused = session({ liveness: null, status_labels: ['fkst-substrate-active'], work_issues: [] });
    const onLoad = vi.fn();
    render(<TabStatus session={paused} observe={idle} onLoadObserve={onLoad} />);
    expect(
      screen.getByText(
        'Live engine details are available while the session is running. It is paused now — no pending work.'
      )
    ).toBeInTheDocument();
    // No fetch affordance, so the slow pod-exec can never be triggered.
    expect(screen.queryByRole('button', { name: 'Live engine details' })).not.toBeInTheDocument();
    expect(onLoad).not.toHaveBeenCalled();
  });

  it('does not reveal an observe snapshot once the pod is no longer live', () => {
    // Even a previously-fetched snapshot is withheld while paused — the gate wins.
    const paused = session({ liveness: null, status_labels: ['fkst-substrate-active'], work_issues: [] });
    render(
      <TabStatus
        session={paused}
        observe={{ status: 'loaded', snapshot: { queues: [{ queue: 'events', depth: 3 }] } }}
        onLoadObserve={() => {}}
      />
    );
    expect(screen.queryByText('events')).not.toBeInTheDocument();
    expect(
      screen.getByText(
        'Live engine details are available while the session is running. It is paused now — no pending work.'
      )
    ).toBeInTheDocument();
  });

  it('shows the slow-note + spinner while observe is loading', () => {
    render(
      <TabStatus session={session()} observe={{ status: 'loading' }} onLoadObserve={() => {}} />
    );
    expect(screen.getByText('Loading engine details…')).toBeInTheDocument();
    expect(
      screen.getByText('This runs inside the session pod — it may take up to a minute.')
    ).toBeInTheDocument();
  });

  it('renders queues + codex-run count once observe has loaded', () => {
    render(
      <TabStatus
        session={session()}
        observe={{
          status: 'loaded',
          snapshot: { queues: [{ queue: 'events', depth: 3, in_flight: 1 }], deliveries: [{}, {}] },
        }}
        onLoadObserve={() => {}}
      />
    );
    expect(screen.getByText('events')).toBeInTheDocument();
    expect(screen.getByText('2 deliveries pending')).toBeInTheDocument();
  });

  it('shows the transient observe error with a retry (defensive fallback)', () => {
    // The session is live here, so the observe section renders; a status-less
    // error maps to the generic "available while running" fallback + a retry.
    render(
      <TabStatus session={session()} observe={{ status: 'error' }} onLoadObserve={() => {}} />
    );
    expect(
      screen.getByText('Live engine details are available while the session is running.')
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Refresh' })).toBeInTheDocument();
  });

  it('explains a 409 observe error as no durable delivery store, without a retry', () => {
    render(
      <TabStatus
        session={session()}
        observe={{ status: 'error', httpStatus: 409 }}
        onLoadObserve={() => {}}
      />
    );
    expect(
      screen.getByText('This session has no durable delivery store to observe.')
    ).toBeInTheDocument();
    // A 409 cannot recover on retry, so no retry button is offered.
    expect(screen.queryByRole('button', { name: 'Refresh' })).not.toBeInTheDocument();
  });

  it('renders duplicately-named queues without a React key collision (bug B3)', () => {
    // The observe payload is untrusted engine JSON: two queues can carry the
    // same `queue` name. A name-only key would collide and React would warn;
    // the fix appends the positional index so both rows key uniquely.
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    render(
      <TabStatus
        session={session()}
        observe={{
          status: 'loaded',
          snapshot: {
            queues: [
              { queue: 'events', depth: 1 },
              { queue: 'events', depth: 4 },
            ],
          },
        }}
        onLoadObserve={() => {}}
      />
    );
    // Both duplicate-named rows render.
    expect(screen.getAllByText('events')).toHaveLength(2);
    // And no "same key" warning was emitted.
    const keyWarned = errorSpy.mock.calls.some((args) =>
      args.some((a) => typeof a === 'string' && /same key/i.test(a))
    );
    expect(keyWarned).toBe(false);
  });

  it('reveals the fetched snapshot when observe resolves from loading to loaded', () => {
    const { rerender } = render(
      <TabStatus session={session()} observe={{ status: 'loading' }} onLoadObserve={() => {}} />
    );
    expect(screen.getByText('Loading engine details…')).toBeInTheDocument();
    rerender(
      <TabStatus
        session={session()}
        observe={{
          status: 'loaded',
          snapshot: { queues: [{ queue: 'events', depth: 3 }] },
        }}
        onLoadObserve={() => {}}
      />
    );
    // The crossfade mounts the loaded body immediately (popLayout), so the
    // queue is present without waiting on an exit animation.
    expect(screen.getByText('events')).toBeInTheDocument();
  });
});
