import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { AuthProvider } from '@/lib/auth/github-auth';
import type { HealthReport, SessionHealth, StalenessState } from '@/lib/api/health';
import { TabHealth, type HealthState } from './tab-health';

function jsonResponse(body: unknown, status = 200): Response {
  return { ok: status >= 200 && status < 300, status, json: async () => body } as Response;
}

const NEWER = 'ns-8f2c1d64-0a1b-4c2d-8e3f-0123456789ab-health-agent-status-report-20260730-141500';
const OLDER = 'ns-8f2c1d64-0a1b-4c2d-8e3f-0123456789ab-health-agent-status-report-20260730-140500';

function summary(id: string, generated_at: string, status: SessionHealth['reports'][0]['status']) {
  return {
    id,
    generated_at,
    status,
    status_raw: status,
    headline: id === NEWER ? 'nothing moved in 10m' : 'was working earlier',
    producer: 'fkst-health@0.1.0',
  };
}

function listing(over: Partial<SessionHealth> = {}, state: StalenessState = 'fresh'): SessionHealth {
  const reports = [
    summary(NEWER, '2026-07-30T14:15:00Z', 'stalled'),
    summary(OLDER, '2026-07-30T14:05:00Z', 'working'),
  ];
  return {
    session_id: 'sess-1',
    reports,
    latest: reports[0]!,
    staleness: { state, expected_interval_secs: 600, age_secs: 2100 },
    ...over,
  };
}

function report(over: Partial<HealthReport> = {}): HealthReport {
  return {
    session_id: 'sess-1',
    id: NEWER,
    generated_at: '2026-07-30T14:15:00Z',
    window_start: '2026-07-30T14:05:00Z',
    status: 'stalled',
    status_raw: 'stalled',
    headline: 'nothing moved in 10m',
    producer: 'fkst-health@0.1.0',
    confidence: 'high',
    expected_interval_secs: 600,
    evidence: [{ key: 'codex_runs_started', value: '0' }],
    work_items: [{ number: 812, state: 'open', progress: 'none' }],
    body_markdown: '## What this session is doing\n\nNothing observable.',
    ...over,
  };
}

function loaded(health: SessionHealth): HealthState {
  return { status: 'loaded', health };
}

function renderTab(state: HealthState, onRetry = vi.fn()) {
  return render(
    <AuthProvider>
      <TabHealth sessionId="sess-1" state={state} onRetry={onRetry} />
    </AuthProvider>
  );
}

describe('TabHealth', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.location.hash = '';
    window.localStorage.setItem('fkst-gh-access', 'ghu_x');
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes(`/health/${OLDER}`)) return jsonResponse(report({ id: OLDER, body_markdown: 'older body' }));
        if (url.includes('/health/')) return jsonResponse(report());
        throw new Error(`unexpected fetch: ${url}`);
      })
    );
  });
  afterEach(() => vi.unstubAllGlobals());

  it('renders the current assessment, its evidence, and the report body', async () => {
    renderTab(loaded(listing()));

    // The headline appears in the card AND in its history row; the card's is the <p>.
    expect(await screen.findByText('nothing moved in 10m', { selector: 'p' })).toBeInTheDocument();
    expect(screen.getAllByText('Stalled', { selector: '.rounded-chip' }).length).toBeGreaterThan(0);
    expect(screen.getByText('fkst-health@0.1.0')).toBeInTheDocument();
    expect(await screen.findByText('codex_runs_started')).toBeInTheDocument();
    expect(await screen.findByText('What this session is doing')).toBeInTheDocument();
    expect(screen.getByText('high')).toBeInTheDocument();
  });

  it('states both the expected interval and the actual age in the stale callout', async () => {
    renderTab(loaded(listing({}, 'stale')));

    const notice = await screen.findByRole('status');
    expect(notice).toHaveTextContent('This session has stopped reporting');
    expect(notice).toHaveTextContent('every 10 min');
    expect(notice).toHaveTextContent('35 min ago');
  });

  /** THE false-alarm regression: a reaped pod is normal, not a fault. */
  it('shows NO stale callout for not_running even with an ancient report', async () => {
    renderTab(
      loaded(
        listing(
          { staleness: { state: 'not_running', expected_interval_secs: 600, age_secs: 99_999 } },
          'not_running'
        )
      )
    );

    expect(await screen.findByText('nothing moved in 10m', { selector: 'p' })).toBeInTheDocument();
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
    expect(screen.queryByText(/stopped reporting/)).not.toBeInTheDocument();
  });

  it('still lists the past reports for a not_running session', async () => {
    renderTab(
      loaded(
        listing(
          { staleness: { state: 'not_running', expected_interval_secs: 600, age_secs: 99_999 } },
          'not_running'
        )
      )
    );
    const history = await screen.findByRole('list', { name: 'Health report history' });
    expect(history.querySelectorAll('li')).toHaveLength(2);
  });

  it('renders a calm empty state when nothing has been reported yet', async () => {
    renderTab(
      loaded({
        session_id: 'sess-1',
        reports: [],
        latest: null,
        staleness: { state: 'never_reported' },
      })
    );
    expect(
      await screen.findByText(/No health report yet\. The first one is due/)
    ).toBeInTheDocument();
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
  });

  it('renders a calm empty state for a not-running session with no history', async () => {
    renderTab(
      loaded({
        session_id: 'sess-1',
        reports: [],
        latest: null,
        staleness: { state: 'not_running' },
      })
    );
    expect(await screen.findByText(/not currently running, so it is not reporting/)).toBeInTheDocument();
  });

  it('omits the evidence section entirely when there is none', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => jsonResponse(report({ evidence: [] })))
    );
    renderTab(loaded(listing()));
    expect(await screen.findByText('What this session is doing')).toBeInTheDocument();
    expect(screen.queryByText('Evidence')).not.toBeInTheDocument();
  });

  it('loads the selected history entry into the body panel', async () => {
    const user = userEvent.setup();
    renderTab(loaded(listing()));
    expect(await screen.findByText('What this session is doing')).toBeInTheDocument();

    await user.click(screen.getByText('was working earlier'));
    await waitFor(() => expect(screen.getByText('older body')).toBeInTheDocument());
  });

  it('renders injected HTML in the report body as inert text', async () => {
    const hostile = '<script>window.__pwned = true;</script>\n\n<img src=x onerror="window.__pwned=true">';
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => jsonResponse(report({ body_markdown: hostile })))
    );
    const { container } = renderTab(loaded(listing()));

    await waitFor(() =>
      expect(screen.getByRole('region', { name: 'Session health assessment' })).toBeInTheDocument()
    );
    expect(container.querySelector('script')).toBeNull();
    expect(container.querySelector('img')).toBeNull();
    // It survives as escaped literal text, so nothing is silently swallowed either.
    expect(screen.getByRole('region', { name: 'Session health assessment' })).toHaveTextContent(
      '<script>'
    );
    expect((window as unknown as { __pwned?: boolean }).__pwned).toBeUndefined();
  });

  it('renders the 503 deployment state distinctly from a transient failure', async () => {
    renderTab({ status: 'error', httpStatus: 503 });
    expect(
      screen.getByText('Health reporting is not configured for this deployment.')
    ).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Try again' })).not.toBeInTheDocument();
  });

  it('offers a retry for a non-503 failure', async () => {
    const user = userEvent.setup();
    const onRetry = vi.fn();
    renderTab({ status: 'error', httpStatus: 403 }, onRetry);

    expect(screen.getByText('Could not load this session’s health.')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Try again' }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it('shows a loading state while the listing is in flight', () => {
    renderTab({ status: 'loading' });
    expect(screen.getByText('Loading health…')).toBeInTheDocument();
  });
});
