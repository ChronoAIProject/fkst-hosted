import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import {
  ALICE_ROW,
  LEGACY_SANDBOX,
  LIFECYCLE_ROW,
  RUNNING_SANDBOX,
  activityPage,
  currentSearch,
  jsonResponse,
  renderOperations,
  sandboxSnapshot,
  seedAuth,
  seedUrl,
  stubOperations,
} from './operations-test-kit';

// The `/operations` behavioural suite: discovery, both views, filters,
// pagination, freshness, and the four distinct "nothing to show" states.
// Identity and scope enforcement live in `operations.security.test.tsx`.

beforeEach(() => {
  window.localStorage.clear();
  seedUrl();
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
  window.history.replaceState(null, '', '/operations');
});

describe('operations route access', () => {
  it('prompts sign-in for an unauthenticated visitor without calling the API', () => {
    const { fetchMock } = stubOperations({});
    renderOperations();
    expect(screen.getByText('Sign in to view operations')).toBeInTheDocument();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('gives a regular authenticated user data, never a denied page', async () => {
    seedAuth();
    stubOperations({ activity: () => jsonResponse(activityPage()) });
    renderOperations();
    expect(await screen.findByTestId('activity-row')).toBeInTheDocument();
    expect(screen.getByTestId('operations-scope')).toHaveTextContent('My activity');
  });

  it('writes the fail-closed personal scope into the URL before any request', async () => {
    seedAuth();
    const { calls } = stubOperations({ activity: () => jsonResponse(activityPage()) });
    renderOperations();
    await screen.findByTestId('activity-row');
    expect(currentSearch().get('scope')).toBe('mine');
    // Every issued request stated that scope explicitly.
    expect(calls.every((call) => call.params.get('scope') === 'mine')).toBe(true);
  });
});

describe('activity view', () => {
  beforeEach(seedAuth);

  it('renders each column of an API-request row from its own contract', async () => {
    stubOperations({ activity: () => jsonResponse(activityPage()) });
    renderOperations();
    const row = await screen.findByTestId('activity-row');
    expect(within(row).getByText('API request')).toBeInTheDocument();
    expect(within(row).getByText('@alice')).toBeInTheDocument();
    expect(within(row).getByText('User token')).toBeInTheDocument();
    expect(within(row).getByText('GET')).toBeInTheDocument();
    expect(within(row).getByText('canvas_overview')).toBeInTheDocument();
    expect(within(row).getByText('/api/v1/overview')).toBeInTheDocument();
    expect(within(row).getByText('limit=20')).toBeInTheDocument();
    expect(within(row).getByText('200')).toBeInTheDocument();
    expect(within(row).getByText('Success')).toBeInTheDocument();
    expect(within(row).getByText('42ms')).toBeInTheDocument();
    expect(within(row).getByText('sess-1')).toBeInTheDocument();
    expect(within(row).getByText('Verified')).toBeInTheDocument();
    expect(within(row).getByText('req-alice-1')).toBeInTheDocument();
  });

  it('never fabricates HTTP values on a lifecycle row', async () => {
    stubOperations({
      activity: () =>
        jsonResponse(activityPage({ items: [LIFECYCLE_ROW], effective_scope: 'mine' })),
    });
    seedUrl('?tab=activity&scope=mine&record_kind=all&session_id=sess-1');
    renderOperations();
    const row = await screen.findByTestId('activity-row');
    expect(within(row).getByText('Lifecycle')).toBeInTheDocument();
    expect(within(row).getByText('Created')).toBeInTheDocument();
    expect(within(row).getByText('System')).toBeInTheDocument();
    // No method, no status, no duration — three em dashes stand in their place.
    expect(within(row).queryByText('GET')).not.toBeInTheDocument();
    expect(within(row).queryByText('200')).not.toBeInTheDocument();
    expect(within(row).getAllByText('—').length).toBeGreaterThanOrEqual(3);
  });

  it('opens typed details from a real control inside the row', async () => {
    stubOperations({ activity: () => jsonResponse(activityPage()) });
    renderOperations();
    const row = await screen.findByTestId('activity-row');
    // The row itself keeps its native `row` role so the column headers stay
    // associated with its cells; the affordance is a button inside a cell, which
    // is what makes the details keyboard-openable.
    expect(row).not.toHaveAttribute('role');
    const open = within(row).getByRole('button', { name: /Details$/ });
    fireEvent.click(open);
    const details = await screen.findByTestId('operations-details');
    expect(within(details).getByText('Actor id')).toBeInTheDocument();
    expect(within(details).getByText('7')).toBeInTheDocument();
    expect(within(details).getByText('GET /api/v1/overview')).toBeInTheDocument();
    // The safe arguments are discrete fields here, not the table's summary.
    expect(within(details).getByText('limit')).toBeInTheDocument();
  });

  it('encodes each filter into the request and the URL', async () => {
    const { calls } = stubOperations({ activity: () => jsonResponse(activityPage()) });
    renderOperations();
    await screen.findByTestId('activity-row');

    fireEvent.change(screen.getByLabelText('Time range'), { target: { value: '7d' } });
    fireEvent.change(screen.getByLabelText('Method'), { target: { value: 'POST' } });
    fireEvent.change(screen.getByLabelText('Status class'), { target: { value: '5xx' } });
    fireEvent.change(screen.getByLabelText('Outcome'), { target: { value: 'timeout' } });

    await waitFor(() => expect(currentSearch().get('method')).toBe('POST'));
    expect(currentSearch().get('range')).toBe('7d');
    expect(currentSearch().get('status_class')).toBe('5xx');
    expect(currentSearch().get('outcome')).toBe('timeout');

    const last = calls[calls.length - 1]!;
    expect(last.params.get('method')).toBe('POST');
    expect(last.params.get('status_class')).toBe('5xx');
    expect(last.params.get('outcome')).toBe('timeout');
  });

  it('debounces a text filter and never issues an unparseable value', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { calls } = stubOperations({ activity: () => jsonResponse(activityPage()) });
    renderOperations();
    await screen.findByTestId('activity-row');
    const before = calls.length;

    const field = screen.getByLabelText('Session id');
    fireEvent.change(field, { target: { value: 'not a session/id' } });
    await vi.advanceTimersByTimeAsync(600);
    // Unparseable: no request, and the field is marked.
    expect(calls.length).toBe(before);
    expect(field).toHaveAttribute('aria-invalid', 'true');

    fireEvent.change(field, { target: { value: 'sess-9' } });
    await vi.advanceTimersByTimeAsync(600);
    await waitFor(() => expect(currentSearch().get('session_id')).toBe('sess-9'));
  });

  it('resets every filter back to the default', async () => {
    stubOperations({ activity: () => jsonResponse(activityPage()) });
    seedUrl('?tab=activity&scope=mine&method=POST&status_class=5xx');
    renderOperations();
    await screen.findByTestId('activity-row');
    fireEvent.click(screen.getByRole('button', { name: 'Reset filters' }));
    await waitFor(() => expect(currentSearch().has('method')).toBe(false));
    expect(currentSearch().has('status_class')).toBe(false);
  });

  it('withholds a personal lifecycle query until an exact session is named', async () => {
    const { calls } = stubOperations({ activity: () => jsonResponse(activityPage()) });
    seedUrl('?tab=activity&scope=mine&record_kind=all');
    renderOperations();
    expect(await screen.findByTestId('activity-session-required')).toBeInTheDocument();
    // The backend would refuse this; the UI never asks.
    expect(calls).toHaveLength(0);
  });

  it('surfaces the ignored parameters of a malformed link', async () => {
    stubOperations({ activity: () => jsonResponse(activityPage()) });
    seedUrl('?tab=activity&scope=mine&method=TRACE&status_code=999');
    renderOperations();
    const notice = await screen.findByTestId('operations-ignored-params');
    expect(notice).toHaveTextContent('method');
    expect(notice).toHaveTextContent('status_code');
  });
});

describe('activity pagination', () => {
  beforeEach(seedAuth);

  it('loads an older page with the server cursor and suspends live refresh', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const older = { ...ALICE_ROW, event_id: 'ev-older-1' };
    const { calls } = stubOperations({
      activity: (params) =>
        jsonResponse(
          params.get('cursor')
            ? activityPage({ items: [older] })
            : activityPage({ next_cursor: 'cursor-1' })
        ),
    });
    renderOperations();
    await screen.findByTestId('activity-row');

    fireEvent.click(screen.getByRole('button', { name: 'Load older' }));
    await waitFor(() => expect(screen.getAllByTestId('activity-row')).toHaveLength(2));
    // A cursor page states no window: the server keeps the one the cursor was
    // issued for, and restating a drifted `now` would get it refused.
    const cursorCall = calls.find((call) => call.params.get('cursor') === 'cursor-1');
    expect(cursorCall).toBeDefined();
    expect(cursorCall!.params.has('from')).toBe(false);

    expect(screen.getByTestId('activity-poll-paused')).toBeInTheDocument();
    const settled = calls.length;
    await vi.advanceTimersByTimeAsync(45_000);
    // The open investigation is not discarded by a background poll.
    expect(calls.length).toBe(settled);
    expect(screen.getAllByTestId('activity-row')).toHaveLength(2);
  });

  it('drops every accumulated page when the server refuses the cursor', async () => {
    const { calls } = stubOperations({
      activity: (params) =>
        params.get('cursor')
          ? jsonResponse({ error: 'invalid_activity_cursor' }, 400)
          : jsonResponse(activityPage({ next_cursor: 'cursor-1' })),
    });
    renderOperations();
    await screen.findByTestId('activity-row');
    fireEvent.click(screen.getByRole('button', { name: 'Load older' }));
    await waitFor(() =>
      expect(screen.getByText(/This page link expired/)).toBeInTheDocument()
    );
    expect(screen.getAllByTestId('activity-row')).toHaveLength(1);
    expect(calls.some((call) => call.params.get('cursor') === 'cursor-1')).toBe(true);
  });

  it('resets pagination when a filter changes', async () => {
    stubOperations({
      activity: (params) =>
        jsonResponse(
          params.get('cursor')
            ? activityPage({ items: [{ ...ALICE_ROW, event_id: 'ev-older-1' }] })
            : activityPage({ next_cursor: 'cursor-1' })
        ),
    });
    renderOperations();
    await screen.findByTestId('activity-row');
    fireEvent.click(screen.getByRole('button', { name: 'Load older' }));
    await waitFor(() => expect(screen.getAllByTestId('activity-row')).toHaveLength(2));

    fireEvent.change(screen.getByLabelText('Method'), { target: { value: 'GET' } });
    await waitFor(() => expect(screen.getAllByTestId('activity-row')).toHaveLength(1));
    expect(screen.queryByTestId('activity-poll-paused')).not.toBeInTheDocument();
  });
});

describe('sandbox view', () => {
  beforeEach(seedAuth);

  it('renders the runtime columns, and never guesses a missing value', async () => {
    stubOperations({
      sandboxes: () => jsonResponse(sandboxSnapshot({ items: [RUNNING_SANDBOX, LEGACY_SANDBOX] })),
    });
    seedUrl('?tab=sandboxes&scope=accessible');
    renderOperations();
    const rows = await screen.findAllByTestId('sandbox-row');
    expect(rows).toHaveLength(2);

    expect(within(rows[0]!).getByText('Running')).toBeInTheDocument();
    expect(within(rows[0]!).getByText('@alice')).toBeInTheDocument();
    expect(within(rows[0]!).getByText('acme/app')).toBeInTheDocument();
    expect(within(rows[0]!).getByText('2')).toBeInTheDocument();

    // The legacy runtime: no creator to guess, no ceiling to count down, and no
    // restart concept to report as zero.
    expect(within(rows[1]!).getByText('Unknown (legacy runtime)')).toBeInTheDocument();
    expect(within(rows[1]!).getByText('Unlimited')).toBeInTheDocument();
    expect(within(rows[1]!).getByText('Not reported')).toBeInTheDocument();
    // An unlimited lifetime renders NO countdown at all — "0s left" would say
    // the opposite of what the deployment configured.
    expect(within(rows[1]!).queryByText(/left$/)).not.toBeInTheDocument();
    expect(within(rows[1]!).queryByText('Expired')).not.toBeInTheDocument();
  });

  it('shows the raw backend state alongside the normalized one, and no URL', async () => {
    stubOperations({ sandboxes: () => jsonResponse(sandboxSnapshot({ items: [LEGACY_SANDBOX] })) });
    seedUrl('?tab=sandboxes&scope=accessible');
    renderOperations();
    fireEvent.click(await screen.findByTestId('sandbox-row'));
    const details = await screen.findByTestId('operations-details');
    expect(within(details).getByText('Running')).toBeInTheDocument();
    expect(within(details).getByText('ACTIVE')).toBeInTheDocument();
    expect(within(details).getByText('Backend state')).toBeInTheDocument();
    // `backend_location` came in as a query-bearing URL and is reduced to its
    // authority; the token can never reach the DOM.
    expect(within(details).getByText('sandbox.example')).toBeInTheDocument();
    expect(details.textContent).not.toContain('secret');
    expect(details.textContent).not.toContain('https://');
  });

  it('flags an attribution conflict visibly', async () => {
    const conflicted = { ...RUNNING_SANDBOX, attribution_source: 'conflict' };
    stubOperations({ sandboxes: () => jsonResponse(sandboxSnapshot({ items: [conflicted] })) });
    seedUrl('?tab=sandboxes&scope=accessible');
    renderOperations();
    const row = await screen.findByTestId('sandbox-row');
    expect(within(row).getByText('Attribution conflict')).toBeInTheDocument();
  });

  it('marks a snapshot stale once it is older than fifteen seconds', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.setSystemTime(Date.parse('2026-08-01T10:00:03.000Z'));
    stubOperations({ sandboxes: () => jsonResponse(sandboxSnapshot()) });
    seedUrl('?tab=sandboxes&scope=accessible');
    renderOperations();
    await screen.findByTestId('sandbox-row');
    expect(screen.queryByTestId('sandbox-stale')).not.toBeInTheDocument();

    // Staleness is measured against the BACKEND's observed_at, not arrival.
    await vi.advanceTimersByTimeAsync(20_000);
    await waitFor(() => expect(screen.getByTestId('sandbox-stale')).toBeInTheDocument());
    // Last-good rows survive: they are still what was last observed.
    expect(screen.getByTestId('sandbox-row')).toBeInTheDocument();
  });

  it('cross-links a session into an own-plus-lifecycle activity query', async () => {
    stubOperations({
      sandboxes: () => jsonResponse(sandboxSnapshot()),
      activity: () => jsonResponse(activityPage({ items: [ALICE_ROW, LIFECYCLE_ROW] })),
    });
    seedUrl('?tab=sandboxes&scope=accessible');
    renderOperations();
    fireEvent.click(await screen.findByTestId('sandbox-row'));
    fireEvent.click(await screen.findByRole('button', { name: 'View activity for this session' }));

    await waitFor(() => expect(currentSearch().get('tab')).toBe('activity'));
    expect(currentSearch().get('session_id')).toBe('sess-1');
    expect(currentSearch().get('record_kind')).toBe('all');
    // A personal scope, so the server returns the caller's own calls plus this
    // session's system lifecycle rows — and nothing else.
    expect(currentSearch().get('scope')).toBe('mine');
    await waitFor(() => expect(screen.getAllByTestId('activity-row')).toHaveLength(2));
  });
});

describe('empty, partial, and outage states are distinguishable', () => {
  beforeEach(seedAuth);

  it('shows a plain sentence for a COMPLETE empty activity page', async () => {
    stubOperations({ activity: () => jsonResponse(activityPage({ items: [] })) });
    renderOperations();
    expect(await screen.findByTestId('operations-empty')).toHaveTextContent(
      'No records match these filters in this window.'
    );
    expect(screen.queryByTestId('activity-partial')).not.toBeInTheDocument();
  });

  it('names the source that could not answer on a PARTIAL page', async () => {
    stubOperations({
      activity: () =>
        jsonResponse(
          activityPage({
            items: [],
            source_status: {
              posthog: 'unavailable',
              relay: 'healthy',
              partial: true,
              message_code: 'posthog_unavailable',
            },
          })
        ),
    });
    renderOperations();
    expect(await screen.findByTestId('activity-partial')).toHaveTextContent(
      'The analytics source could not answer'
    );
    // A page a source could not fill has ZERO rows for a reason that is not "no
    // records matched" — it must never borrow the complete-empty copy.
    expect(screen.getByTestId('operations-incomplete')).toHaveTextContent(
      'This page is incomplete'
    );
    expect(screen.queryByTestId('operations-empty')).not.toBeInTheDocument();
    expect(screen.queryByText('No records match these filters in this window.')).not.toBeInTheDocument();
  });

  it('explains a withheld custom-range query instead of claiming nothing matched', async () => {
    const { calls } = stubOperations({ activity: () => jsonResponse(activityPage()) });
    seedUrl('?tab=activity&scope=mine&range=custom');
    renderOperations();
    // Selecting the custom preset before naming both bounds issues no request…
    expect(await screen.findByTestId('activity-window-required')).toHaveTextContent(
      'Enter both UTC bounds'
    );
    // …so the panel must not report a result the deployment never produced.
    expect(screen.queryByTestId('operations-empty')).not.toBeInTheDocument();
    expect(calls).toHaveLength(0);
  });

  it('refuses a window this deployment says is too wide, before the request', async () => {
    // The deployment states a 7-day ceiling; the 30d preset is now unqueryable
    // even though the client's own default would have allowed it.
    const { calls } = stubOperations({
      activity: () => jsonResponse(activityPage({ max_range_days: 7 })),
    });
    renderOperations();
    await screen.findByTestId('activity-row');

    fireEvent.change(screen.getByLabelText('Time range'), { target: { value: '30d' } });
    expect(await screen.findByTestId('activity-window-required')).toHaveTextContent(
      'wider than the 7 days this deployment allows'
    );
    expect(screen.queryByTestId('operations-empty')).not.toBeInTheDocument();
    // Not one request the server was guaranteed to answer with a 400. Asserted on
    // the WINDOW each call carried rather than on a total call count: the count
    // is perturbed by any unrelated request still in flight when it is sampled,
    // which is a property of the harness, not of the behaviour under test.
    const windowDays = (params: URLSearchParams) => {
      const from = params.get('from');
      const to = params.get('to');
      if (!from || !to) return 0;
      return (Date.parse(to) - Date.parse(from)) / 86_400_000;
    };
    const overWide = calls.filter(
      (call) => call.path.endsWith('/operations/activity') && windowDays(call.params) > 7
    );
    expect(overWide).toHaveLength(0);
  });

  it('reports a cold session-visibility projection as a failure, not an empty fleet', async () => {
    stubOperations({
      sandboxes: () => jsonResponse({ error: 'session_visibility_unavailable' }, 503),
    });
    seedUrl('?tab=sandboxes&scope=accessible');
    renderOperations();
    const error = await screen.findByTestId('operations-error');
    expect(error).toHaveTextContent('Session visibility is still recovering');
    expect(screen.queryByTestId('operations-empty')).not.toBeInTheDocument();
  });

  it('keeps an activity failure out of the sandbox view, and the reverse', async () => {
    stubOperations({
      activity: () => jsonResponse({ error: 'audit_query_not_configured' }, 503),
      sandboxes: () => jsonResponse(sandboxSnapshot()),
    });
    renderOperations();
    expect(await screen.findByTestId('operations-error')).toHaveTextContent(
      'no activity query configured'
    );

    // The runtime feed is untouched by the analytics outage.
    fireEvent.click(screen.getByRole('tab', { name: 'Sandboxes' }));
    expect(await screen.findByTestId('sandbox-row')).toBeInTheDocument();
    expect(screen.queryByTestId('operations-error')).not.toBeInTheDocument();
  });

  it('offers a retry that re-issues the request', async () => {
    let fail = true;
    const { calls } = stubOperations({
      activity: () => (fail ? jsonResponse({ error: 'unavailable' }, 503) : jsonResponse(activityPage())),
    });
    renderOperations();
    await screen.findByTestId('operations-error');
    const before = calls.length;
    fail = false;
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    await waitFor(() => expect(screen.getByTestId('activity-row')).toBeInTheDocument());
    expect(calls.length).toBeGreaterThan(before);
  });
});
