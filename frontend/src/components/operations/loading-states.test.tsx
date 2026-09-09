import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import { DEFAULT_ACTIVITY_FILTERS, DEFAULT_SANDBOX_FILTERS } from '@/lib/operations/state';
import type { ActivityFeed } from '@/lib/hooks/use-operations-activity';
import type { SandboxFeed } from '@/lib/hooks/use-operations-sandboxes';
import { ActivityView } from './activity-view';
import { SandboxView } from './sandbox-view';

/// The Operations panels used to render an EMPTY TABLE during their first load:
/// control fell through to the rows branch with `rows: []`, so "we are still
/// asking" was indistinguishable from "nothing matched". Loading is a third
/// distinct shape, and it must be gated on the first request only — never on
/// `refreshing`, which is true on every 15-second poll and would blank the table
/// under a reader mid-investigation.

const activityFeed = (over: Partial<ActivityFeed> = {}): ActivityFeed => ({
  page: null,
  rows: [],
  error: null,
  loading: false,
  refreshing: false,
  updatedAt: null,
  hasMore: false,
  loadingOlder: false,
  olderError: null,
  pollSuspended: false,
  loadOlder: vi.fn(),
  refresh: vi.fn(),
  ...over,
});

const sandboxFeed = (over: Partial<SandboxFeed> = {}): SandboxFeed => ({
  inventory: null,
  error: null,
  loading: false,
  refreshing: false,
  updatedAt: null,
  refresh: vi.fn(),
  ...over,
});

function renderActivity(feed: ActivityFeed) {
  return render(
    <ActivityView
      feed={feed}
      filters={DEFAULT_ACTIVITY_FILTERS}
      showActorFilters={false}
      sessionRequired={false}
      windowIssue={null}
      maxRangeDays={30}
      onFiltersChange={vi.fn()}
      onReset={vi.fn()}
    />
  );
}

function renderSandboxes(feed: SandboxFeed) {
  return render(
    <SandboxView
      feed={feed}
      filters={DEFAULT_SANDBOX_FILTERS}
      onFiltersChange={vi.fn()}
      onReset={vi.fn()}
      onViewActivity={vi.fn()}
    />
  );
}

describe('ActivityView loading state', () => {
  it('says it is still asking instead of rendering an empty table', () => {
    renderActivity(activityFeed({ loading: true }));
    const region = screen.getByTestId('operations-loading-activity');
    expect(region).toHaveTextContent('Loading activity…');
    // …and explains the wait rather than leaving a bare spinner.
    expect(region).toHaveTextContent(/can take a moment/);
  });

  it('never dresses the wait as an empty result', () => {
    renderActivity(activityFeed({ loading: true }));
    expect(
      screen.queryByText('No records match these filters in this window.')
    ).not.toBeInTheDocument();
  });

  it('shows the empty result, not a spinner, once the answer is in', () => {
    renderActivity(activityFeed({ loading: false }));
    expect(screen.queryByTestId('operations-loading-activity')).not.toBeInTheDocument();
    expect(screen.getByText('No records match these filters in this window.')).toBeInTheDocument();
  });

  it('keeps rows on screen while a poll refreshes them', () => {
    // `refreshing` is true on every 15-second poll. Gating the loading state on
    // it would blank the table under the reader; only the FIRST request with
    // nothing to show may take over the panel.
    renderActivity(activityFeed({ refreshing: true, loading: false }));
    expect(screen.queryByTestId('operations-loading-activity')).not.toBeInTheDocument();
  });
});

describe('SandboxView loading state', () => {
  it('says it is still asking instead of rendering an empty fleet', () => {
    renderSandboxes(sandboxFeed({ loading: true }));
    const region = screen.getByTestId('operations-loading-sandboxes');
    expect(region).toHaveTextContent('Loading live sandboxes…');
    expect(region).toHaveTextContent(/can take a moment/);
  });

  it('never dresses the wait as an empty fleet', () => {
    renderSandboxes(sandboxFeed({ loading: true }));
    expect(screen.queryByText('No live sandboxes match these filters.')).not.toBeInTheDocument();
  });

  it('shows the empty fleet, not a spinner, once the answer is in', () => {
    renderSandboxes(sandboxFeed({ loading: false }));
    expect(screen.queryByTestId('operations-loading-sandboxes')).not.toBeInTheDocument();
    expect(screen.getByText('No live sandboxes match these filters.')).toBeInTheDocument();
  });

  it('keeps the snapshot on screen while a poll refreshes it', () => {
    renderSandboxes(sandboxFeed({ refreshing: true, loading: false }));
    expect(screen.queryByTestId('operations-loading-sandboxes')).not.toBeInTheDocument();
  });
});
