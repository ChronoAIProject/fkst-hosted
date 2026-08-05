import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { BrowserRouter } from 'react-router-dom';
import { AuthProvider } from '@/lib/auth/github-auth';
import { Workflows } from './workflows';

// The `/workflows` behavioural suite.
//
// The fetch stub answers ONLY the schedules routes and throws on anything else,
// which is the point: a test that starts calling GitHub directly from the
// browser fails loudly instead of quietly passing.

const NOW = Date.parse('2026-08-01T00:00:00Z');

const summary = (overrides: Record<string, unknown> = {}) => ({
  scheduleIssue: 50,
  title: 'nightly sourcing',
  htmlUrl: 'https://github.com/acme/site/issues/50',
  workflowId: 'sourcing',
  runMode: 'cron: 0 1 * * 1-5',
  cadence: 'weekdays at 01:00 UTC',
  state: 'idle',
  nextDue: '2026-08-01T03:00:00Z',
  lastRun: null,
  successRate30d: null,
  invalidDetail: null,
  ...overrides,
});

const run = (overrides: Record<string, unknown> = {}) => ({
  slot: '2026-07-31T01:00:00Z',
  manual: false,
  status: 'ok',
  startedAt: '2026-07-31T01:00:00Z',
  endedAt: '2026-07-31T01:12:00Z',
  durationS: 720,
  issue: 4242,
  detail: null,
  ...overrides,
});

function json(body: unknown, status = 200) {
  return {
    ok: status >= 200 && status < 300,
    status,
    headers: new Headers(),
    json: async () => body,
  } as Response;
}

type Handler = (url: URL, init?: RequestInit) => Response;

function stub(handler: Handler) {
  const calls: { path: string; method: string }[] = [];
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = new URL(String(input), 'http://localhost');
    calls.push({ path: url.pathname, method: init?.method ?? 'GET' });
    if (!url.pathname.includes('/schedules')) {
      throw new Error(`unexpected fetch: ${url.pathname}`);
    }
    return handler(url, init);
  });
  vi.stubGlobal('fetch', fetchMock);
  return { fetchMock, calls };
}

function renderPage(search = '?repo=acme/site') {
  window.history.replaceState(null, '', `/workflows${search}`);
  return render(
    <AuthProvider>
      {/* BrowserRouter, not MemoryRouter: the page's whole navigational state
          lives in the query string, and only a browser router puts it somewhere
          a test can assert the way a user would read it. */}
      <BrowserRouter>
        <Workflows />
      </BrowserRouter>
    </AuthProvider>
  );
}

beforeEach(() => {
  window.localStorage.clear();
  window.localStorage.setItem('fkst-gh-access', 'test-access-token');
  vi.setSystemTime(NOW);
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
  window.history.replaceState(null, '', '/workflows');
});

describe('/workflows', () => {
  it('lists a repository’s schedules with cadence, state and next run', async () => {
    stub(() => json({ owner: 'acme', name: 'site', installed: true, schedules: [summary()] }));
    renderPage();
    expect(await screen.findByText('sourcing')).toBeInTheDocument();
    expect(screen.getByText('weekdays at 01:00 UTC')).toBeInTheDocument();
    expect(screen.getByTestId('lifecycle-idle')).toBeInTheDocument();
    // Three hours out, rendered as a coarse distance rather than a raw instant.
    expect(screen.getByText('in 3h')).toBeInTheDocument();
  });

  it('shows an invalid schedule’s reason INLINE in the list', async () => {
    // A schedule that has silently stopped is the failure this surface exists to
    // catch; it would be invisible if the reason were only on the detail page.
    stub(() =>
      json({
        owner: 'acme',
        name: 'site',
        installed: true,
        schedules: [
          summary({ state: 'invalid', invalidDetail: 'missing required section `### Run Mode`' }),
        ],
      })
    );
    renderPage();
    expect(await screen.findByTestId('invalid-detail-50')).toHaveTextContent('### Run Mode');
    expect(screen.getByTestId('lifecycle-invalid')).toBeInTheDocument();
  });

  it('explains an uninstalled repository instead of showing an empty list', async () => {
    stub(() => json({ owner: 'acme', name: 'site', installed: false, schedules: [] }));
    renderPage();
    expect(await screen.findByText(/app is not installed/i)).toBeInTheDocument();
  });

  it('offers the issue template when a repository has no schedules yet', async () => {
    stub(() => json({ owner: 'acme', name: 'site', installed: true, schedules: [] }));
    renderPage();
    const link = await screen.findByRole('link', { name: /template/i });
    expect(link).toHaveAttribute(
      'href',
      'https://github.com/acme/site/issues/new?template=fkst-scheduled-workflow.md'
    );
  });

  it('opens a schedule into the URL, so the view is a shareable link', async () => {
    stub((url) => {
      if (url.pathname.endsWith('/schedules')) {
        return json({ owner: 'acme', name: 'site', installed: true, schedules: [summary()] });
      }
      return json({ summary: summary(), upcoming: [], arguments: {}, runs: [] });
    });
    renderPage();
    fireEvent.click(await screen.findByText('sourcing'));
    await waitFor(() =>
      expect(new URLSearchParams(window.location.search).get('schedule')).toBe('50')
    );
    expect(await screen.findByTestId('schedule-detail')).toBeInTheDocument();
  });

  it('renders the next firings and the arguments on the detail view', async () => {
    stub((url) => {
      if (url.pathname.endsWith('/schedules')) {
        return json({ owner: 'acme', name: 'site', installed: true, schedules: [summary()] });
      }
      return json({
        summary: summary(),
        upcoming: ['2026-08-01T03:00:00Z', '2026-08-02T03:00:00Z'],
        arguments: { role: 'engineer', min_score: '6' },
        runs: [],
      });
    });
    renderPage('?repo=acme/site&schedule=50');
    expect(await screen.findByTestId('upcoming')).toBeInTheDocument();
    expect(screen.getByTestId('arguments')).toHaveTextContent('engineer');
    // The one line that explains why there is no inline cadence editor.
    expect(screen.getByText(/no editor here on purpose/i)).toBeInTheDocument();
  });

  it('expands a run into its per-step outcomes, including the step that never ran', async () => {
    stub((url) => {
      if (url.pathname.endsWith('/schedules')) {
        return json({ owner: 'acme', name: 'site', installed: true, schedules: [summary()] });
      }
      if (url.pathname.includes('/runs/')) {
        return json({
          run: run({ status: 'failed' }),
          steps: [
            { index: 1, id: 'scrape', status: 'ok', durationS: 41 },
            { index: 2, id: 'score', status: 'failed', durationS: 9 },
            { index: 3, id: 'publish', status: 'skipped', durationS: null },
          ],
          runIssue: 4242,
        });
      }
      return json({ summary: summary(), upcoming: [], arguments: {}, runs: [run()] });
    });
    renderPage('?repo=acme/site&schedule=50');
    fireEvent.click(await screen.findByTestId('run-row-2026-07-31T01:00:00Z'));
    expect(await screen.findByTestId('run-stepper')).toBeInTheDocument();
    expect(screen.getByTestId('step-1')).toHaveTextContent('scrape');
    expect(screen.getByTestId('step-3')).toHaveTextContent('publish');
    expect(screen.getByTestId('step-status-skipped')).toBeInTheDocument();
    expect(screen.getByText(/#4242/)).toBeInTheDocument();
  });

  it('disables run-now while a run is already in flight', async () => {
    // The server answers 409 either way; saying so first is better than inviting
    // a click that always fails.
    stub((url) => {
      if (url.pathname.endsWith('/schedules')) {
        return json({
          owner: 'acme',
          name: 'site',
          installed: true,
          schedules: [summary({ state: 'running' })],
        });
      }
      return json({
        summary: summary({ state: 'running' }),
        upcoming: [],
        arguments: {},
        runs: [],
      });
    });
    renderPage('?repo=acme/site&schedule=50');
    expect(await screen.findByTestId('action-run-now')).toBeDisabled();
  });

  it('toggles pause into resume from the schedule’s own state', async () => {
    const { calls } = stub((url, init) => {
      if (url.pathname.endsWith('/pause')) return new Response(null, { status: 204 });
      if (url.pathname.endsWith('/schedules')) {
        return json({ owner: 'acme', name: 'site', installed: true, schedules: [summary()] });
      }
      void init;
      return json({ summary: summary(), upcoming: [], arguments: {}, runs: [] });
    });
    renderPage('?repo=acme/site&schedule=50');
    fireEvent.click(await screen.findByTestId('action-pause-resume'));
    await waitFor(() =>
      expect(calls.some((call) => call.path.endsWith('/pause') && call.method === 'POST')).toBe(true)
    );
  });

  it('shows a paused schedule the resume action instead', async () => {
    stub((url) => {
      if (url.pathname.endsWith('/schedules')) {
        return json({
          owner: 'acme',
          name: 'site',
          installed: true,
          schedules: [summary({ state: 'paused' })],
        });
      }
      return json({
        summary: summary({ state: 'paused' }),
        upcoming: [],
        arguments: {},
        runs: [],
      });
    });
    renderPage('?repo=acme/site&schedule=50');
    expect(await screen.findByTestId('action-pause-resume')).toHaveTextContent(/resume/i);
  });

  it('surfaces the server’s own refusal message verbatim', async () => {
    stub((url) => {
      if (url.pathname.endsWith('/run')) {
        return json({ message: 'a run is already in flight for this schedule' }, 409);
      }
      if (url.pathname.endsWith('/schedules')) {
        return json({ owner: 'acme', name: 'site', installed: true, schedules: [summary()] });
      }
      return json({ summary: summary(), upcoming: [], arguments: {}, runs: [] });
    });
    renderPage('?repo=acme/site&schedule=50');
    fireEvent.click(await screen.findByTestId('action-run-now'));
    expect(await screen.findByTestId('action-error')).toHaveTextContent(
      'a run is already in flight for this schedule'
    );
  });

  it('offers a retry rather than a blank page when the read fails', async () => {
    stub(() => json({ message: 'boom' }, 502));
    renderPage();
    expect(await screen.findByRole('button', { name: /try again/i })).toBeInTheDocument();
  });

  it('never talks to GitHub from the browser', async () => {
    // The stub throws on any non-schedules path; a passing render is the proof.
    const { calls } = stub(() =>
      json({ owner: 'acme', name: 'site', installed: true, schedules: [summary()] })
    );
    renderPage();
    await screen.findByText('sourcing');
    expect(calls.every((call) => call.path.startsWith('/api/v1/repos/'))).toBe(true);
  });
});
