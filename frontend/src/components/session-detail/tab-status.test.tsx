import { afterEach, describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { IssueDetail, SessionDetail } from '@/lib/api/types';
import { TabStatus } from './tab-status';
import type { ObserveState } from './observe-state';

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
    expect(screen.getByText('No work items yet.')).toBeInTheDocument();
  });

  it('offers the Live engine details button and fires the callback', async () => {
    const user = userEvent.setup();
    const onLoad = vi.fn();
    render(<TabStatus session={session()} observe={idle} onLoadObserve={onLoad} />);
    await user.click(screen.getByRole('button', { name: 'Live engine details' }));
    expect(onLoad).toHaveBeenCalledTimes(1);
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

  it('shows the error state with a retry', () => {
    render(
      <TabStatus session={session()} observe={{ status: 'error' }} onLoadObserve={() => {}} />
    );
    expect(screen.getByText('Could not load the live engine details.')).toBeInTheDocument();
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
