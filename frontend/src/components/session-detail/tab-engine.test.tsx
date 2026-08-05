import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { IssueDetail, SessionDetail, SessionRecoveryProjection } from '@/lib/api/types';
import type { ObserveState } from './observe-state';
import { TabEngine } from './tab-engine';

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

const idle: ObserveState = { status: 'idle' };

/** A paused session: a latched active label whose pod was reaped, no open work. */
const paused = () =>
  session({ liveness: null, status_labels: ['fkst-substrate-active'], work_issues: [] });

describe('TabEngine', () => {
  // ---- activation is the request (#5841) -----------------------------------

  it('fetches the engine snapshot on activation, because opening the tab IS the request', () => {
    const onLoad = vi.fn();
    render(<TabEngine session={session()} observe={idle} onLoadObserve={onLoad} />);
    expect(onLoad).toHaveBeenCalledTimes(1);
  });

  it('never auto-fetches while the pod is not live', () => {
    // The gate protects a pod exec, so auto-load must sit behind it too.
    const onLoad = vi.fn();
    render(<TabEngine session={paused()} observe={idle} onLoadObserve={onLoad} />);
    expect(onLoad).not.toHaveBeenCalled();
  });

  it('does not re-fire on re-render, and never retries an error automatically', () => {
    // A minute-long pod exec looping on failure is the worst outcome this
    // surface could produce; recovery is the explicit Refresh button only.
    const onLoad = vi.fn();
    const { rerender } = render(
      <TabEngine session={session()} observe={idle} onLoadObserve={onLoad} />
    );
    expect(onLoad).toHaveBeenCalledTimes(1);

    rerender(
      <TabEngine session={session()} observe={{ status: 'loading' }} onLoadObserve={onLoad} />
    );
    rerender(
      <TabEngine session={session()} observe={{ status: 'error' }} onLoadObserve={onLoad} />
    );
    rerender(<TabEngine session={session()} observe={idle} onLoadObserve={onLoad} />);
    expect(onLoad).toHaveBeenCalledTimes(1);
  });

  // ---- the liveness gate ----------------------------------------------------

  it('uses projected runtime, not stale legacy liveness, for the live-engine gate', () => {
    const onLoad = vi.fn();
    const { rerender } = render(
      <TabEngine
        session={session({
          liveness: 'live',
          recovery: recovery({ state: 'recovering', reason: 'runtime_absent', runtime: 'absent' }),
        })}
        observe={idle}
        onLoadObserve={onLoad}
      />
    );
    // Stale `liveness: 'live'` must not re-enable the exec after an
    // authoritative absent observation: no fetch, no affordance.
    expect(onLoad).not.toHaveBeenCalled();
    expect(screen.queryByRole('button', { name: 'Live engine details' })).not.toBeInTheDocument();

    rerender(
      <TabEngine
        session={session({ liveness: null, recovery: recovery() })}
        observe={idle}
        onLoadObserve={onLoad}
      />
    );
    expect(onLoad).toHaveBeenCalledTimes(1);
  });

  it('gates the live engine on a live pod: paused note, no fetch button when idle', () => {
    const onLoad = vi.fn();
    render(<TabEngine session={paused()} observe={idle} onLoadObserve={onLoad} />);
    expect(
      screen.getByText(
        'Live engine details are available while the session is running. It is paused now — no pending work.'
      )
    ).toBeInTheDocument();
    // No fetch affordance, so the slow pod-exec can never be triggered.
    expect(screen.queryByRole('button', { name: 'Live engine details' })).not.toBeInTheDocument();
    expect(onLoad).not.toHaveBeenCalled();
  });

  it('explains a recovering runtime instead of offering a fetch', () => {
    // Distinct from paused: the pod is coming back, so the copy says "after the
    // recovering runtime is live" rather than "it is paused now".
    const onLoad = vi.fn();
    render(
      <TabEngine
        session={session({
          liveness: null,
          recovery: recovery({
            state: 'recovering',
            reason: 'runtime_absent',
            open_work_items: 2,
            runtime: 'absent',
          }),
        })}
        observe={idle}
        onLoadObserve={onLoad}
      />
    );
    expect(
      screen.getByText(
        'Live engine details will be available after the recovering runtime is live.'
      )
    ).toBeInTheDocument();
    expect(onLoad).not.toHaveBeenCalled();
  });

  it('does not reveal an observe snapshot once the pod is no longer live', () => {
    // Even a previously-fetched snapshot is withheld while paused — the gate wins.
    render(
      <TabEngine
        session={paused()}
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

  it('keeps the manual affordance as a fallback while the state is still idle', async () => {
    // The effect runs after the first paint, so the button is what a reader sees
    // for that frame — and it stays the honest fallback if the auto-load were
    // ever prevented. Clicking it must fetch, not sit inert.
    const user = userEvent.setup();
    const onLoad = vi.fn();
    render(<TabEngine session={session()} observe={idle} onLoadObserve={onLoad} />);
    const button = screen.getByRole('button', { name: 'Live engine details' });
    expect(onLoad).toHaveBeenCalledTimes(1); // the auto-load
    await user.click(button);
    expect(onLoad).toHaveBeenCalledTimes(2); // …plus the explicit click
  });

  // ---- observe states -------------------------------------------------------

  it('shows the slow-note + spinner while observe is loading', () => {
    render(
      <TabEngine session={session()} observe={{ status: 'loading' }} onLoadObserve={() => {}} />
    );
    expect(screen.getByText('Loading engine details…')).toBeInTheDocument();
    expect(
      screen.getByText('This runs inside the session pod — it may take up to a minute.')
    ).toBeInTheDocument();
  });

  it('renders queues + codex-run count once observe has loaded', () => {
    render(
      <TabEngine
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
      <TabEngine session={session()} observe={{ status: 'error' }} onLoadObserve={() => {}} />
    );
    expect(
      screen.getByText('Live engine details are available while the session is running.')
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Refresh' })).toBeInTheDocument();
  });

  it('explains a 409 observe error as no durable delivery store, without a retry', () => {
    render(
      <TabEngine
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
      <TabEngine
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
    expect(screen.getAllByText('events')).toHaveLength(2);
    const keyWarned = errorSpy.mock.calls.some((args) =>
      args.some((a) => typeof a === 'string' && /same key/i.test(a))
    );
    expect(keyWarned).toBe(false);
    errorSpy.mockRestore();
  });

  it('reveals the fetched snapshot when observe resolves from loading to loaded', () => {
    const { rerender } = render(
      <TabEngine session={session()} observe={{ status: 'loading' }} onLoadObserve={() => {}} />
    );
    expect(screen.getByText('Loading engine details…')).toBeInTheDocument();
    rerender(
      <TabEngine
        session={session()}
        observe={{ status: 'loaded', snapshot: { queues: [{ queue: 'events', depth: 3 }] } }}
        onLoadObserve={() => {}}
      />
    );
    // The crossfade mounts the loaded body immediately (popLayout), so the
    // queue is present without waiting on an exit animation.
    expect(screen.getByText('events')).toBeInTheDocument();
  });
});
