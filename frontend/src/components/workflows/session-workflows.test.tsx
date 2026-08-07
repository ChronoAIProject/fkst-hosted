import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { AuthProvider } from '@/lib/auth/github-auth';
import type {
  RepoSchedulesResponse,
  RunSummary,
  ScheduleDetail,
  ScheduleRunDetail,
  ScheduleSummary,
} from '@/lib/api/schedules';
import { SessionWorkflows } from './session-workflows';

const summary = (over: Partial<ScheduleSummary> & Pick<ScheduleSummary, 'scheduleIssue'>) =>
  ({
    title: 'nightly sourcing',
    htmlUrl: `https://github.com/acme/site/issues/${over.scheduleIssue}`,
    workflowId: 'github-candidate-sourcing',
    runMode: 'cron: 0 1 * * 1-5',
    cadence: 'weekdays at 01:00 UTC',
    state: 'idle',
    creator: 'shining',
    nextDue: '2099-01-01T01:00:00Z',
    lastRun: null,
    successRate30d: 0.75,
    invalidDetail: null,
    ...over,
  }) as ScheduleSummary;

const run = (over: Partial<RunSummary> & Pick<RunSummary, 'slot'>): RunSummary => ({
  manual: false,
  status: 'ok',
  startedAt: over.slot,
  endedAt: '2026-08-04T01:12:00Z',
  durationS: 720,
  elapsedS: null,
  issue: 4242,
  detail: null,
  ...over,
});

const detail = (over: Partial<ScheduleDetail> = {}): ScheduleDetail => ({
  summary: summary({ scheduleIssue: 50 }),
  upcoming: ['2099-01-01T01:00:00Z'],
  arguments: { role: 'AI Tools Application Engineer' },
  runs: [run({ slot: '2026-08-04T01:00:00Z' })],
  latestRun: {
    run: run({ slot: '2026-08-04T01:00:00Z' }),
    steps: [
      { index: 1, id: 'scrape', status: 'ok', durationS: 41 },
      { index: 2, id: 'score', status: 'failed', durationS: 9 },
      { index: 3, id: 'publish', status: 'skipped', durationS: null },
    ],
    runIssue: 4242,
  },
  ...over,
});

/** Route the SPA's three schedule reads to fixtures; anything else is a 404 so a
 *  stray call is visible rather than silent. */
function stubApi(routes: {
  list?: RepoSchedulesResponse;
  listStatus?: number;
  detail?: ScheduleDetail;
  runDetail?: ScheduleRunDetail;
  onMutate?: (action: string) => { status: number; body: unknown };
}) {
  const calls: string[] = [];
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    calls.push(url);
    const json = (body: unknown, status = 200) =>
      new Response(JSON.stringify(body), {
        status,
        headers: { 'content-type': 'application/json' },
      });
    if (init?.method === 'POST') {
      const action = url.split('/').pop() ?? '';
      const result = routes.onMutate?.(action) ?? { status: 202, body: 999 };
      return json(result.body, result.status);
    }
    if (/\/schedules\/\d+\/runs\//.test(url)) return json(routes.runDetail ?? null);
    if (/\/schedules\/\d+$/.test(url)) return json(routes.detail ?? detail());
    if (/\/schedules$/.test(url)) {
      if (routes.listStatus && routes.listStatus !== 200) {
        return json({ message: 'boom' }, routes.listStatus);
      }
      return json(
        routes.list ?? {
          owner: 'acme',
          name: 'site',
          installed: true,
          schedules: [summary({ scheduleIssue: 50 })],
        }
      );
    }
    return json({ message: `no fixture for ${url}` }, 404);
  });
  vi.stubGlobal('fetch', fetchMock);
  return { calls, fetchMock };
}

const renderTab = (creator = 'shining') =>
  render(
    <AuthProvider>
      <SessionWorkflows owner="acme" name="site" creator={creator} />
    </AuthProvider>
  );

describe('SessionWorkflows', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.localStorage.setItem('fkst-gh-access', 'ghu_x');
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it('lists only the schedules routed to THIS session', async () => {
    // A repository may host several creators' sessions. A schedule assigned to
    // someone else can never run for this one, so it is not this session's to
    // show — and mixing them is exactly what moving the surface here fixes.
    stubApi({
      list: {
        owner: 'acme',
        name: 'site',
        installed: true,
        schedules: [
          summary({ scheduleIssue: 50, workflowId: 'mine' }),
          summary({ scheduleIssue: 51, workflowId: 'someone-elses', creator: 'other-dev' }),
        ],
      },
    });
    renderTab();

    expect(await screen.findByTestId('schedule-row-50')).toBeInTheDocument();
    expect(screen.queryByTestId('schedule-row-51')).not.toBeInTheDocument();
  });

  it('matches the creator case-insensitively, as GitHub treats logins', async () => {
    stubApi({
      list: {
        owner: 'acme',
        name: 'site',
        installed: true,
        schedules: [summary({ scheduleIssue: 50, creator: 'ChronoAI-Shining' })],
      },
    });
    renderTab('chronoai-shining');

    expect(await screen.findByTestId('schedule-row-50')).toBeInTheDocument();
  });

  it('fills the detail pane from the first schedule without waiting for a click', async () => {
    stubApi({});
    renderTab();

    expect(await screen.findByTestId('schedule-detail')).toBeInTheDocument();
    expect(screen.getByTestId('arguments')).toHaveTextContent('AI Tools Application Engineer');
  });

  it('shows the most recent run’s per-step outcomes with no second selection', async () => {
    // The whole point: the steps ride on the schedule detail, so the stepper is
    // there on arrival rather than two clicks deep.
    stubApi({});
    renderTab();

    const latest = await screen.findByTestId('latest-run');
    expect(within(latest).getByTestId('step-1')).toHaveTextContent('scrape');
    expect(within(latest).getByTestId('step-2')).toHaveTextContent('score');
    expect(within(latest).getByTestId('step-3')).toHaveTextContent('publish');
    expect(within(latest).getByTestId('step-status-skipped')).toBeInTheDocument();
  });

  it('shows an in-flight run’s age and its run issue, and says the steps are not in yet', async () => {
    // Mid-run the runner has posted nothing — it writes one record at the end —
    // so "recorded no per-step outcomes" would be a lie about a run that simply
    // has not finished.
    stubApi({
      detail: detail({
        summary: summary({ scheduleIssue: 50, state: 'running' }),
        runs: [run({ slot: '2026-08-05T01:00:00Z', status: 'dispatched' })],
        latestRun: {
          run: run({
            slot: '2026-08-05T01:00:00Z',
            status: 'dispatched',
            endedAt: null,
            durationS: null,
            elapsedS: 125,
          }),
          steps: [],
          runIssue: 4242,
        },
      }),
    });
    renderTab();

    const latest = await screen.findByTestId('latest-run');
    expect(within(latest).getByTestId('latest-run-timing')).toHaveTextContent('running for 2m 5s');
    expect(within(latest).getByTestId('run-issue-link')).toHaveAttribute(
      'href',
      'https://github.com/acme/site/issues/4242'
    );
    expect(within(latest).getByTestId('run-stepper')).toHaveTextContent(
      'Awaiting the first step record'
    );
    expect(screen.queryByText(/recorded no per-step outcomes/)).not.toBeInTheDocument();
    // Run-now is refused server-side while a run is in flight, so the button
    // says so first rather than inviting a click that always 409s.
    expect(screen.getByTestId('action-run-now')).toBeDisabled();
  });

  it('keeps the selected schedule when a reload reorders the list', async () => {
    // Selection is stored by ISSUE NUMBER, so a response that returns the same
    // schedules in a different order cannot swap the detail pane out from under
    // the reader.
    const list: RepoSchedulesResponse = {
      owner: 'acme',
      name: 'site',
      installed: true,
      schedules: [
        summary({ scheduleIssue: 50, workflowId: 'alpha' }),
        summary({ scheduleIssue: 51, workflowId: 'beta' }),
        summary({ scheduleIssue: 52, workflowId: 'gamma' }),
      ],
    };
    stubApi({ list });
    renderTab();

    await userEvent.click(await screen.findByTestId('schedule-row-51'));
    expect(screen.getByTestId('schedule-row-51')).toHaveAttribute('aria-current', 'true');

    // A pause reloads the list. Serve it in a different order, as a later read
    // legitimately may. The new order is chosen so every wrong implementation
    // lands somewhere else: an INDEX would now show #50, and a selection that
    // was simply lost would fall back to the new first row, #52.
    list.schedules = [list.schedules[2]!, list.schedules[0]!, list.schedules[1]!];
    await userEvent.click(screen.getByTestId('action-pause-resume'));

    expect(await screen.findByTestId('schedule-row-51')).toHaveAttribute('aria-current', 'true');
    expect(screen.getByTestId('schedule-row-50')).toHaveAttribute('aria-current', 'false');
    expect(screen.getByTestId('schedule-row-52')).toHaveAttribute('aria-current', 'false');
  });

  it('lists a schedule that routes to no session instead of hiding it', async () => {
    // Zero or several assignees means no session will ever run it. Once the
    // repository-level list is gone, omitting it would delete it from the
    // product while it silently never fires.
    stubApi({
      list: {
        owner: 'acme',
        name: 'site',
        installed: true,
        schedules: [
          summary({ scheduleIssue: 50 }),
          summary({ scheduleIssue: 52, workflowId: 'orphan', creator: null }),
        ],
      },
    });
    renderTab();

    const unrouted = await screen.findByTestId('unrouted-schedules');
    expect(within(unrouted).getByTestId('unrouted-row-52')).toHaveAttribute(
      'href',
      'https://github.com/acme/site/issues/52'
    );
    // It is a link out, not a selectable row: the fix is assigning one session
    // creator on GitHub, and no lifecycle or firing time is claimed for it.
    expect(screen.queryByTestId('schedule-row-52')).not.toBeInTheDocument();
  });

  it('expands an earlier run into its own steps, and never repeats the latest one', async () => {
    stubApi({
      detail: detail({
        runs: [
          run({ slot: '2026-08-04T01:00:00Z' }),
          run({ slot: '2026-08-03T01:00:00Z', status: 'failed' }),
        ],
      }),
      runDetail: {
        run: run({ slot: '2026-08-03T01:00:00Z', status: 'failed' }),
        steps: [{ index: 1, id: 'scrape', status: 'failed', durationS: 3 }],
        runIssue: 4200,
      },
    });
    renderTab();

    const history = await screen.findByTestId('run-history');
    expect(within(history).queryByTestId('run-row-2026-08-04T01:00:00Z')).not.toBeInTheDocument();

    await userEvent.click(within(history).getByTestId('run-row-2026-08-03T01:00:00Z'));
    expect(await within(history).findByTestId('step-1')).toHaveTextContent('scrape');
  });

  it('surfaces a refused mutation’s own message rather than a generic failure', async () => {
    stubApi({
      onMutate: () => ({
        status: 409,
        body: { message: 'a run is already in flight for this schedule' },
      }),
    });
    renderTab();

    await userEvent.click(await screen.findByTestId('action-run-now'));
    expect(await screen.findByTestId('action-error')).toHaveTextContent(
      'a run is already in flight for this schedule'
    );
  });

  it('says the session has no schedules rather than showing an empty rail', async () => {
    stubApi({ list: { owner: 'acme', name: 'site', installed: true, schedules: [] } });
    renderTab();

    expect(await screen.findByText('This session has no scheduled workflows')).toBeInTheDocument();
  });

  it('offers a retry when the list cannot be read', async () => {
    const { fetchMock } = stubApi({ listStatus: 500 });
    renderTab();

    expect(
      await screen.findByText('Could not load the scheduled workflows for this repository.')
    ).toBeInTheDocument();
    fetchMock.mockClear();
    await userEvent.click(screen.getByRole('button', { name: 'Try again' }));
    expect(fetchMock).toHaveBeenCalled();
  });

  it('owns a scroll region in every state, loaded or not', async () => {
    // The session-detail panel hands a master/detail tab a fixed BOX, not a
    // scroller — so this tab has to supply one in each of its short states too,
    // or a long error or empty-state explanation is clipped with no way to reach
    // it. Asserted inside this component rather than through the drawer's tab
    // loop, where the crossfade briefly leaves the PREVIOUS tab's scroller in
    // the tree and would pass for a tab that owns none.
    const cases: Array<[string, Parameters<typeof stubApi>[0]]> = [
      ['error', { listStatus: 500 }],
      ['not installed', { list: { owner: 'acme', name: 'site', installed: false, schedules: [] } }],
      ['empty', { list: { owner: 'acme', name: 'site', installed: true, schedules: [] } }],
      ['loaded', {}],
    ];
    for (const [label, routes] of cases) {
      stubApi(routes);
      const { unmount } = renderTab();
      const body =
        label === 'loaded'
          ? await screen.findByTestId('schedule-detail')
          : await screen.findByTestId('session-workflows');
      expect(
        body.querySelector('.overflow-y-auto') ?? body.closest('.overflow-y-auto'),
        `the ${label} state must own a scroll region`
      ).not.toBeNull();
      unmount();
      vi.unstubAllGlobals();
    }
  });

  it('reports an uninstalled repository as a fact, not as a failure', async () => {
    stubApi({ list: { owner: 'acme', name: 'site', installed: false, schedules: [] } });
    renderTab();

    expect(
      await screen.findByText(
        'The FKST app is not installed on this repository, or you cannot see it.'
      )
    ).toBeInTheDocument();
  });
});
