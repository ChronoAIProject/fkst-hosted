import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, screen, waitFor } from '@testing-library/react';
import {
  ANON_ROW,
  BOB_ROW,
  ALICE_ROW,
  activityPage,
  currentSearch,
  jsonResponse,
  renderOperations,
  sandboxSnapshot,
  seedAuth,
  seedUrl,
  stubOperations,
} from './operations-test-kit';

// The `/operations` authorization suite.
//
// Every assertion here is about one rule: the SERVER decides what this caller
// may see, and no client state — a URL, a cached page, a previous response, a
// hidden control — may widen, preserve, or imply that decision.

beforeEach(() => {
  window.localStorage.clear();
  seedUrl();
  seedAuth();
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
  window.history.replaceState(null, '', '/operations');
});

/** Drive the document's visibility, which the poll observes. */
function setHidden(hidden: boolean) {
  Object.defineProperty(document, 'hidden', { configurable: true, value: hidden });
  document.dispatchEvent(new Event('visibilitychange'));
}

const globalPage = (over: Record<string, unknown> = {}) =>
  activityPage({ effective_scope: 'all', can_view_all: true, ...over });

describe('effective scope is the only truth', () => {
  it('offers a regular user no scope control and no actor filters', async () => {
    stubOperations({ activity: () => jsonResponse(activityPage()) });
    renderOperations();
    await screen.findByTestId('activity-row');
    expect(screen.queryByTestId('operations-scope-control')).not.toBeInTheDocument();
    expect(screen.queryByLabelText('Actor id')).not.toBeInTheDocument();
    expect(screen.queryByLabelText('Actor login')).not.toBeInTheDocument();
  });

  it('upgrades a global admin to the all scope and reveals the actor filters', async () => {
    stubOperations({
      activity: (params) =>
        jsonResponse(params.get('scope') === 'all' ? globalPage({ items: [ALICE_ROW, BOB_ROW, ANON_ROW] }) : activityPage({ can_view_all: true })),
    });
    renderOperations();
    await waitFor(() => expect(currentSearch().get('scope')).toBe('all'));
    expect(screen.getByTestId('operations-scope')).toHaveTextContent('All activity');
    expect(await screen.findByTestId('operations-scope-control')).toBeInTheDocument();
    expect(screen.getByLabelText('Actor id')).toBeInTheDocument();
    // An administrator sees other actors AND the unattributed records.
    await waitFor(() => expect(screen.getAllByTestId('activity-row')).toHaveLength(3));
    expect(screen.getByText('@bob')).toBeInTheDocument();
    expect(screen.getByText('Anonymous')).toBeInTheDocument();
  });

  it('lets a global admin switch to their personal scope, clearing actor filters', async () => {
    stubOperations({
      activity: (params) =>
        jsonResponse(
          params.get('scope') === 'all'
            ? globalPage({ items: [ALICE_ROW, BOB_ROW] })
            : activityPage({ can_view_all: true })
        ),
    });
    seedUrl('?tab=activity&scope=all&actor_id=8');
    renderOperations();
    await screen.findByTestId('operations-scope-control');

    fireEvent.click(screen.getByRole('tab', { name: 'Mine' }));
    await waitFor(() => expect(currentSearch().get('scope')).toBe('mine'));
    // A personal scope may not carry a cross-actor filter at all.
    expect(currentSearch().has('actor_id')).toBe(false);
    expect(screen.queryByLabelText('Actor id')).not.toBeInTheDocument();
  });
});

describe('a crafted global URL cannot reveal anything', () => {
  it('never renders a global row, and normalizes the URL to the allowed scope', async () => {
    const seenScopes: string[] = [];
    stubOperations({
      activity: (params) => {
        seenScopes.push(params.get('scope') ?? '');
        return params.get('scope') === 'all'
          ? jsonResponse({ error: 'operations_scope_forbidden' }, 403)
          : jsonResponse(activityPage());
      },
    });
    seedUrl('?tab=activity&scope=all&actor_id=99');
    renderOperations();

    // The denial rewrites the URL and drops the filter only a global caller may
    // carry; the personal query then succeeds.
    await waitFor(() => expect(currentSearch().get('scope')).toBe('mine'));
    expect(currentSearch().has('actor_id')).toBe(false);
    expect(await screen.findByTestId('operations-scope-reset')).toBeInTheDocument();
    expect(await screen.findByTestId('activity-row')).toBeInTheDocument();

    // No global page was ever rendered, and no scope control was ever offered.
    expect(screen.getByTestId('operations-scope')).toHaveTextContent('My activity');
    expect(screen.queryByTestId('operations-scope-control')).not.toBeInTheDocument();
    expect(seenScopes).toEqual(['all', 'mine']);
    expect(screen.queryByText('@bob')).not.toBeInTheDocument();
  });

  it('applies the same reset to the sandbox view', async () => {
    stubOperations({
      sandboxes: (params) =>
        params.get('scope') === 'all'
          ? jsonResponse({ error: 'operations_scope_forbidden' }, 403)
          : jsonResponse(sandboxSnapshot()),
    });
    seedUrl('?tab=sandboxes&scope=all');
    renderOperations();
    await waitFor(() => expect(currentSearch().get('scope')).toBe('accessible'));
    expect(await screen.findByTestId('sandbox-row')).toBeInTheDocument();
    expect(screen.getByTestId('operations-scope')).toHaveTextContent('My accessible sandboxes');
  });
});

describe('a permission downgrade takes effect immediately', () => {
  it('clears the global rows and the scope control on the next refused poll', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    let downgraded = false;
    stubOperations({
      activity: (params) => {
        if (params.get('scope') === 'all') {
          return downgraded
            ? jsonResponse({ error: 'operations_scope_forbidden' }, 403)
            : jsonResponse(globalPage({ items: [ALICE_ROW, BOB_ROW] }));
        }
        return jsonResponse(activityPage({ can_view_all: false }));
      },
    });
    seedUrl('?tab=activity&scope=all');
    renderOperations();
    await waitFor(() => expect(screen.getAllByTestId('activity-row')).toHaveLength(2));
    expect(screen.getByTestId('operations-scope-control')).toBeInTheDocument();

    downgraded = true;
    await vi.advanceTimersByTimeAsync(15_000);

    await waitFor(() => expect(currentSearch().get('scope')).toBe('mine'));
    // Not one global row survives the downgrade.
    expect(screen.queryByText('@bob')).not.toBeInTheDocument();
    await waitFor(() =>
      expect(screen.queryByTestId('operations-scope-control')).not.toBeInTheDocument()
    );
  });
});

describe('a scope mismatch is a hard failure', () => {
  it('renders no rows when the answered scope is not the requested one', async () => {
    stubOperations({
      // The caller asked for `mine`; the server answered `all`. Whose rows are
      // these? Unanswerable, so none of them are shown.
      activity: () => jsonResponse(globalPage({ items: [ALICE_ROW, BOB_ROW] })),
    });
    seedUrl('?tab=activity&scope=mine');
    renderOperations();
    const error = await screen.findByTestId('operations-error');
    expect(error).toHaveTextContent('answered a different scope');
    expect(screen.queryByTestId('activity-row')).not.toBeInTheDocument();
    expect(screen.queryByText('@bob')).not.toBeInTheDocument();
  });

  it('clears rows that were already on screen when a later poll mismatches', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    let mismatch = false;
    stubOperations({
      activity: () =>
        jsonResponse(
          mismatch
            ? // Same request (`mine`), an answer claiming the global scope.
              globalPage({ items: [ALICE_ROW, BOB_ROW] })
            : activityPage({ items: [ALICE_ROW] })
        ),
    });
    seedUrl('?tab=activity&scope=mine');
    renderOperations();
    await screen.findByTestId('activity-row');

    mismatch = true;
    await vi.advanceTimersByTimeAsync(15_000);

    // A hard validation failure is not a staleness banner over surviving rows:
    // the rows go, because we can no longer say whose they are.
    await waitFor(() => expect(screen.queryByTestId('activity-row')).not.toBeInTheDocument());
    expect(screen.getByTestId('operations-error')).toHaveTextContent('answered a different scope');
    expect(screen.queryByTestId('activity-stale')).not.toBeInTheDocument();
    expect(screen.queryByText('@bob')).not.toBeInTheDocument();
  });

  it('clears an already-loaded sandbox snapshot the same way', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    let mismatch = false;
    stubOperations({
      sandboxes: () =>
        jsonResponse(
          mismatch
            ? sandboxSnapshot({ effective_scope: 'all', can_view_all: true })
            : sandboxSnapshot()
        ),
    });
    seedUrl('?tab=sandboxes&scope=accessible');
    renderOperations();
    await screen.findByTestId('sandbox-row');

    mismatch = true;
    await vi.advanceTimersByTimeAsync(5_000);

    await waitFor(() => expect(screen.queryByTestId('sandbox-row')).not.toBeInTheDocument());
    expect(screen.getByTestId('operations-error')).toHaveTextContent('answered a different scope');
    expect(screen.queryByTestId('sandbox-refresh-failed')).not.toBeInTheDocument();
  });

  it('rejects a global page that does not itself claim the capability', async () => {
    stubOperations({
      activity: () =>
        jsonResponse(activityPage({ effective_scope: 'all', can_view_all: false, items: [BOB_ROW] })),
    });
    seedUrl('?tab=activity&scope=all');
    renderOperations();
    await screen.findByTestId('operations-error');
    expect(screen.queryByText('@bob')).not.toBeInTheDocument();
  });
});

describe('identity changes clear everything', () => {
  it('drops the rows the moment the viewer signs out', async () => {
    stubOperations({ activity: () => jsonResponse(activityPage()) });
    renderOperations({ authControls: true });
    await screen.findByTestId('activity-row');

    fireEvent.click(screen.getByRole('button', { name: 'probe-sign-out' }));
    expect(screen.queryByTestId('activity-row')).not.toBeInTheDocument();
    expect(screen.getByText('Sign in to view operations')).toBeInTheDocument();
  });

  it('shows the re-authenticate prompt when a 401 survives a refresh', async () => {
    stubOperations({ activity: () => jsonResponse({ error: 'unauthorized' }, 401) });
    renderOperations();
    // The refresh path has no refresh token to use, so the session is over; the
    // workspace stays mounted with the prompt rather than blanking.
    expect(await screen.findByText('Session expired')).toBeInTheDocument();
  });
});

describe('an unauthorized or unknown session is indistinguishable', () => {
  it('renders one non-enumerating not-found state for an exact session id', async () => {
    stubOperations({
      sandboxes: () => jsonResponse({ error: 'sandbox_not_found' }, 404),
    });
    seedUrl('?tab=sandboxes&scope=accessible&session_id=someone-elses');
    renderOperations();
    const error = await screen.findByTestId('operations-error');
    expect(error).toHaveTextContent('No such session.');
  });

  it('uses that same copy for the activity route', async () => {
    stubOperations({
      activity: () => jsonResponse({ error: 'activity_session_not_found' }, 404),
    });
    seedUrl('?tab=activity&scope=mine&record_kind=all&session_id=someone-elses');
    renderOperations();
    expect(await screen.findByTestId('operations-error')).toHaveTextContent('No such session.');
  });
});

describe('polling cadence', () => {
  it('refreshes activity every 15 seconds while visible and pauses when hidden', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    setHidden(false);
    const { calls } = stubOperations({ activity: () => jsonResponse(activityPage()) });
    renderOperations();
    await screen.findByTestId('activity-row');
    const initial = calls.length;

    await vi.advanceTimersByTimeAsync(15_000);
    expect(calls.length).toBe(initial + 1);

    setHidden(true);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(calls.length).toBe(initial + 1);

    // Returning to the tab refreshes immediately.
    setHidden(false);
    await waitFor(() => expect(calls.length).toBe(initial + 2));
  });

  it('refreshes sandboxes every 5 seconds', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    setHidden(false);
    const { calls } = stubOperations({ sandboxes: () => jsonResponse(sandboxSnapshot()) });
    seedUrl('?tab=sandboxes&scope=accessible');
    renderOperations();
    await screen.findByTestId('sandbox-row');
    const initial = calls.length;
    await vi.advanceTimersByTimeAsync(15_000);
    expect(calls.length).toBe(initial + 3);
  });

  it('polls only the view that is showing', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    setHidden(false);
    const { calls } = stubOperations({
      activity: () => jsonResponse(activityPage()),
      sandboxes: () => jsonResponse(sandboxSnapshot()),
    });
    renderOperations();
    await screen.findByTestId('activity-row');
    await vi.advanceTimersByTimeAsync(15_000);
    expect(calls.every((call) => call.path.endsWith('/activity'))).toBe(true);
  });
});

describe('the browser talks to nothing but the two operations routes', () => {
  it('never calls PostHog, the relay, Kubernetes, or OpenSandbox', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    setHidden(false);
    const { calls } = stubOperations({
      activity: () => jsonResponse(activityPage()),
      sandboxes: () => jsonResponse(sandboxSnapshot()),
    });
    renderOperations();
    await screen.findByTestId('activity-row');
    fireEvent.click(screen.getByRole('tab', { name: 'Sandboxes' }));
    await screen.findByTestId('sandbox-row');
    await vi.advanceTimersByTimeAsync(20_000);

    // The stub throws on any other path; this asserts the positive form too.
    expect(calls.length).toBeGreaterThan(1);
    for (const call of calls) {
      expect(call.path).toMatch(/\/api\/v1\/operations\/(activity|sandboxes)$/);
    }
  });
});

describe('tab keyboard behaviour', () => {
  it('moves selection with ArrowRight/ArrowLeft/Home/End and rovers the tabindex', async () => {
    stubOperations({
      activity: () => jsonResponse(activityPage()),
      sandboxes: () => jsonResponse(sandboxSnapshot()),
    });
    renderOperations();
    await screen.findByTestId('activity-row');

    const tablist = screen.getByTestId('operations-tabs');
    const activity = screen.getByRole('tab', { name: 'Activity' });
    const sandboxes = screen.getByRole('tab', { name: 'Sandboxes' });
    expect(activity).toHaveAttribute('tabindex', '0');
    expect(sandboxes).toHaveAttribute('tabindex', '-1');

    fireEvent.keyDown(tablist, { key: 'ArrowRight' });
    await waitFor(() => expect(sandboxes).toHaveAttribute('aria-selected', 'true'));
    expect(sandboxes).toHaveAttribute('tabindex', '0');
    expect(activity).toHaveAttribute('tabindex', '-1');

    // Wrapping is part of the pattern.
    fireEvent.keyDown(tablist, { key: 'ArrowRight' });
    await waitFor(() => expect(activity).toHaveAttribute('aria-selected', 'true'));

    fireEvent.keyDown(tablist, { key: 'End' });
    await waitFor(() => expect(sandboxes).toHaveAttribute('aria-selected', 'true'));
    fireEvent.keyDown(tablist, { key: 'Home' });
    await waitFor(() => expect(activity).toHaveAttribute('aria-selected', 'true'));
  });

  it('keeps the tabs pointing at the one stable panel', async () => {
    stubOperations({ activity: () => jsonResponse(activityPage()) });
    renderOperations();
    await screen.findByTestId('activity-row');
    const panel = screen.getByRole('tabpanel');
    for (const tab of screen.getAllByRole('tab')) {
      expect(tab).toHaveAttribute('aria-controls', panel.id);
    }
    expect(panel).toHaveAttribute('aria-labelledby', screen.getByRole('tab', { name: 'Activity' }).id);
  });
});
