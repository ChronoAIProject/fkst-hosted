import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { AuthProvider } from '@/lib/auth/github-auth';
import type { IssueDetail, SessionDetail } from '@/lib/api/types';
import { TabLogs } from './tab-logs';

function jsonResponse(body: unknown, status = 200): Response {
  return { ok: status >= 200 && status < 300, status, json: async () => body } as Response;
}

/** Parse the `path` / `tail_bytes` / `run` query of a stubbed fetch URL. */
function query(url: string): URLSearchParams {
  return new URL(url, 'http://x').searchParams;
}

const trigger: IssueDetail = {
  number: 7,
  title: 'sess',
  state: 'open',
  author: 'shining',
  labels: [],
  html_url: 'https://github.com/o/r/issues/7',
  created_at: '2026-07-01T00:00:00Z',
  updated_at: '2026-07-01T00:00:00Z',
  closed_at: null,
};

const session = (over: Partial<SessionDetail> = {}): SessionDetail => ({
  session_id: 'sess-1',
  name: 'nightly',
  creator: 'shining',
  work_label: null,
  auto_merge: null,
  environment: null,
  source_branch: null,
  target_branch: 'fkst-hosted-default',
  packages: [],
  invalid_reason: null,
  status_labels: [],
  trigger,
  work_issues: [],
  log_url: 'https://api.example.test/api/v1/logs/sess-1',
  liveness: null,
  prs: [],
  ...over,
});

// A legacy session's single synthetic run: empty start ⇒ the compact "Latest
// logs" label, no per-run picker. Most tests only care about the file view, so
// they use this to keep the runs layer inert.
const singleLatestRun = [{ run_id: 'latest', started_at: '' }];

// Two real incarnations, newest first: r-2 is still running (no ended_at); r-1
// is a completed window.
const twoRuns = [
  { run_id: 'r-2', started_at: '2026-07-20T02:00:00Z' },
  { run_id: 'r-1', started_at: '2026-07-20T01:00:00Z', ended_at: '2026-07-20T01:30:00Z' },
];

const manifest = {
  session_id: 'sess-1',
  generated_at: '2026-07-19T00:00:00Z',
  files: [
    { path: 'fkst-substrate/driver.log', size: 2048, label: 'Driver' },
    { path: 'fkst-substrate/codex/codex.log', size: 4096, label: 'Codex' },
  ],
};

const fileContent = {
  session_id: 'sess-1',
  path: 'fkst-substrate/driver.log',
  content: 'alpha line\nbeta line\nalpha again',
  total_bytes: 33,
  returned_bytes: 33,
  truncated: false,
};

function renderLogs(over: Partial<SessionDetail> = {}) {
  return render(
    <AuthProvider>
      <TabLogs session={session(over)} />
    </AuthProvider>
  );
}

describe('TabLogs', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.location.hash = '';
    window.localStorage.setItem('fkst-gh-access', 'ghu_x');
  });
  afterEach(() => vi.unstubAllGlobals());

  it('shows the unavailable note when the session has no id', () => {
    render(
      <AuthProvider>
        <TabLogs session={session({ session_id: null })} />
      </AuthProvider>
    );
    expect(screen.getByText('Logs are not available for this session yet.')).toBeInTheDocument();
  });

  it('lists the bundle files, auto-loads the first, and renders its content', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith('/runs')) return jsonResponse(singleLatestRun);
        if (url.includes('/manifest')) return jsonResponse(manifest);
        if (url.includes('/file?')) return jsonResponse(fileContent);
        throw new Error(`unexpected fetch: ${url}`);
      })
    );
    renderLogs();

    // Both file tabs + a download-bundle link.
    expect(await screen.findByText('driver.log')).toBeInTheDocument();
    expect(screen.getByText('codex.log')).toBeInTheDocument();
    expect(screen.getByRole('link', { name: /Download full bundle/ })).toHaveAttribute(
      'href',
      'https://api.example.test/api/v1/logs/sess-1'
    );
    // The first file's content shows in the viewer.
    await waitFor(() => expect(screen.getByText(/beta line/)).toBeInTheDocument());
  });

  it('counts in-file search matches (debounced)', async () => {
    const user = userEvent.setup();
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith('/runs')) return jsonResponse(singleLatestRun);
        if (url.includes('/manifest')) return jsonResponse(manifest);
        return jsonResponse(fileContent);
      })
    );
    renderLogs();
    const search = await screen.findByPlaceholderText('Find in file…');
    await user.type(search, 'alpha');
    // "alpha" appears twice in the fixture; the count settles after the debounce.
    expect(await screen.findByText('2 matches')).toBeInTheDocument();
  });

  it('surfaces a manifest load error and retries on demand', async () => {
    const user = userEvent.setup();
    let manifestCalls = 0;
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith('/runs')) return jsonResponse(singleLatestRun);
        if (url.includes('/manifest')) {
          manifestCalls += 1;
          return manifestCalls === 1 ? jsonResponse(null, 403) : jsonResponse(manifest);
        }
        return jsonResponse(fileContent);
      })
    );
    renderLogs();
    expect(await screen.findByText('Could not load the session logs.')).toBeInTheDocument();

    // Retry re-invokes the manifest fetch and, on success, lists the files.
    await user.click(screen.getByRole('button', { name: 'Retry' }));
    expect(await screen.findByText('driver.log')).toBeInTheDocument();
  });

  it('explains a 503 manifest error as unconfigured log storage', async () => {
    // A 503 from the manifest endpoint means the deployment has no log storage;
    // the copy says so specifically instead of the generic failure line.
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith('/runs')) return jsonResponse(singleLatestRun);
        if (url.includes('/manifest')) return jsonResponse(null, 503);
        return jsonResponse(fileContent);
      })
    );
    renderLogs();
    expect(
      await screen.findByText("Log storage isn't configured for this deployment.")
    ).toBeInTheDocument();
    // The generic failure line is NOT shown for a 503.
    expect(screen.queryByText('Could not load the session logs.')).not.toBeInTheDocument();
  });

  it('drops a stale in-flight response when the file selection changes (B1)', async () => {
    const user = userEvent.setup();
    // The first-selected file (driver) resolves slowly; the newly-selected file
    // (codex) resolves immediately. The late driver response must NOT overwrite
    // the codex content that is already on screen.
    let releaseDriver!: () => void;
    const driverGate = new Promise<void>((resolve) => {
      releaseDriver = resolve;
    });
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith('/runs')) return jsonResponse(singleLatestRun);
        if (url.includes('/manifest')) return jsonResponse(manifest);
        if (url.includes('/file?')) {
          const path = query(url).get('path');
          if (path === 'fkst-substrate/driver.log') {
            await driverGate; // stays pending until the test releases it
            return jsonResponse({ ...fileContent, path, content: 'DRIVER-OLD' });
          }
          return jsonResponse({ ...fileContent, path, content: 'CODEX-NEW' });
        }
        throw new Error(`unexpected fetch: ${url}`);
      })
    );
    renderLogs();

    // Auto-selects driver (its fetch is now parked). Switch to codex.
    const codexTab = await screen.findByRole('tab', { name: /codex\.log/ });
    await user.click(codexTab);
    expect(await screen.findByText('CODEX-NEW')).toBeInTheDocument();

    // Release the stale driver response; it must be discarded, leaving codex.
    releaseDriver();
    await waitFor(() => expect(screen.getByText('CODEX-NEW')).toBeInTheDocument());
    expect(screen.queryByText('DRIVER-OLD')).not.toBeInTheDocument();
  });

  it('keeps the last-good content and flags staleness on a failed Refresh', async () => {
    const user = userEvent.setup();
    let fileCalls = 0;
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith('/runs')) return jsonResponse(singleLatestRun);
        if (url.includes('/manifest')) return jsonResponse(manifest);
        if (url.includes('/file?')) {
          fileCalls += 1;
          return fileCalls === 1 ? jsonResponse(fileContent) : jsonResponse(null, 500);
        }
        throw new Error(`unexpected fetch: ${url}`);
      })
    );
    renderLogs();
    await screen.findByText(/beta line/);

    await user.click(screen.getByRole('button', { name: 'Refresh' }));
    // The failed refresh surfaces the staleness notice but does NOT wipe the
    // content the user was reading.
    expect(
      await screen.findByText('Showing the last loaded content — the refresh failed.')
    ).toBeInTheDocument();
    expect(screen.getByText(/beta line/)).toBeInTheDocument();
  });

  it('caveats the search count on a truncated tail and loads the full file on demand', async () => {
    const user = userEvent.setup();
    const truncated = {
      ...fileContent,
      content: 'alpha\nbeta\nalpha',
      returned_bytes: 200 * 1024,
      total_bytes: 500 * 1024,
      truncated: true,
    };
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith('/runs')) return jsonResponse(singleLatestRun);
        if (url.includes('/manifest')) return jsonResponse(manifest);
        if (url.includes('/file?')) {
          // A tail request returns the truncated view; a full request (no
          // tail_bytes) returns the whole file.
          return query(url).has('tail_bytes')
            ? jsonResponse(truncated)
            : jsonResponse({ ...fileContent, content: 'whole file body here', truncated: false });
        }
        throw new Error(`unexpected fetch: ${url}`);
      })
    );
    renderLogs();

    // Truncation notice + load-full affordance are present.
    expect(await screen.findByRole('button', { name: 'Load full file' })).toBeInTheDocument();

    // The search count is caveated as covering only the shown tail.
    const search = await screen.findByPlaceholderText('Find in file…');
    await user.type(search, 'alpha');
    expect(await screen.findByText(/in the shown tail/)).toBeInTheDocument();

    // Loading the full file replaces the tail; the truncation UI disappears.
    await user.click(screen.getByRole('button', { name: 'Load full file' }));
    await waitFor(() =>
      expect(screen.queryByRole('button', { name: 'Load full file' })).not.toBeInTheDocument()
    );
    expect(screen.getByText(/whole file body here/)).toBeInTheDocument();
  });

  // ---- Per-run picker (issue #568) -----------------------------------------

  it('renders a run picker (newest default) and reloads the bundle when switching runs', async () => {
    const user = userEvent.setup();
    const manifestRuns: (string | null)[] = [];
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith('/runs')) return jsonResponse(twoRuns);
        if (url.includes('/manifest')) {
          manifestRuns.push(query(url).get('run'));
          return jsonResponse(manifest);
        }
        if (url.includes('/file?')) {
          const run = query(url).get('run');
          return jsonResponse({ ...fileContent, content: `content for ${run}` });
        }
        throw new Error(`unexpected fetch: ${url}`);
      })
    );
    renderLogs();

    // The picker defaults to the newest run (r-2), which is still running.
    const select = (await screen.findByRole('combobox')) as HTMLSelectElement;
    expect(select.value).toBe('r-2');
    expect(screen.getByRole('option', { name: /running/ })).toBeInTheDocument();
    // Its bundle loaded with run=r-2.
    await waitFor(() => expect(screen.getByText('content for r-2')).toBeInTheDocument());
    expect(manifestRuns[0]).toBe('r-2');

    // Switching to the older run reloads the manifest + file for that run.
    await user.selectOptions(select, 'r-1');
    await waitFor(() => expect(screen.getByText('content for r-1')).toBeInTheDocument());
    expect(manifestRuns).toContain('r-1');
  });

  it('renders a compact single label (no dropdown) for a legacy synthetic run', async () => {
    const manifestRuns: (string | null)[] = [];
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith('/runs')) return jsonResponse(singleLatestRun);
        if (url.includes('/manifest')) {
          manifestRuns.push(query(url).get('run'));
          return jsonResponse(manifest);
        }
        if (url.includes('/file?')) return jsonResponse(fileContent);
        throw new Error(`unexpected fetch: ${url}`);
      })
    );
    renderLogs();

    // Compact "Latest logs" label, and NO run dropdown.
    expect(await screen.findByText('Latest logs')).toBeInTheDocument();
    expect(screen.queryByRole('combobox')).not.toBeInTheDocument();
    // The synthetic "latest" run carries no run param.
    await screen.findByText('driver.log');
    expect(manifestRuns).toEqual([null]);
  });

  it('falls back to the latest bundle (with a notice) when the run list fails', async () => {
    const manifestRuns: (string | null)[] = [];
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith('/runs')) return jsonResponse(null, 500);
        if (url.includes('/manifest')) {
          manifestRuns.push(query(url).get('run'));
          return jsonResponse(manifest);
        }
        if (url.includes('/file?')) return jsonResponse(fileContent);
        throw new Error(`unexpected fetch: ${url}`);
      })
    );
    renderLogs();

    // A non-blocking notice, no picker, and the latest bundle still loads with
    // no run param.
    expect(
      await screen.findByText("Couldn't load the run list — showing the latest logs.")
    ).toBeInTheDocument();
    expect(screen.queryByRole('combobox')).not.toBeInTheDocument();
    expect(await screen.findByText('driver.log')).toBeInTheDocument();
    expect(manifestRuns).toEqual([null]);
  });

  // #5765: an empty run list means the session has never flushed a log. It must
  // render the neutral empty state and issue NO manifest request -- the old
  // behaviour asked for a manifest that could not exist and showed its 404 as
  // "Unable to load session logs" with a retry that could never succeed.
  it('shows the empty state and requests no manifest when there are no runs', async () => {
    const calls: string[] = [];
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        calls.push(url);
        if (url.endsWith('/runs')) return jsonResponse([]);
        throw new Error(`unexpected fetch: ${url}`);
      })
    );
    renderLogs();

    expect(
      await screen.findByText('No logs yet — this session has not written any.')
    ).toBeInTheDocument();
    await waitFor(() => expect(calls.some((u) => u.endsWith('/runs'))).toBe(true));
    expect(calls.filter((u) => u.includes('/manifest'))).toEqual([]);
    expect(screen.queryByRole('combobox')).not.toBeInTheDocument();
  });

  it('shows the no-storage message when the run list returns 503', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith('/runs')) return jsonResponse(null, 503);
        throw new Error(`unexpected fetch: ${url}`);
      })
    );
    renderLogs();

    expect(
      await screen.findByText("Log storage isn't configured for this deployment.")
    ).toBeInTheDocument();
    // No file view is attempted on a 503.
    expect(screen.queryByText('driver.log')).not.toBeInTheDocument();
  });
});

/// Layout contract (#5840). The Logs tab is master/detail like Health: files on
/// the left rail, the selected file on the right. Before this, the run picker,
/// the file list and the content were stacked, and the tab panel supplied ONE
/// scroller — so a long file list pushed the content off-screen and scrolling to
/// it dragged the file list (the thing being navigated by) out of view.
describe('TabLogs layout', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.localStorage.setItem('fkst-gh-access', 'ghu_x');
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith('/runs')) return jsonResponse(singleLatestRun);
        if (url.includes('/manifest')) return jsonResponse(manifest);
        if (url.includes('/file?')) return jsonResponse(fileContent);
        throw new Error(`unexpected fetch: ${url}`);
      })
    );
  });
  afterEach(() => vi.unstubAllGlobals());

  it('puts the files in a rail beside the selected file, in one equal-height grid', async () => {
    const { container } = renderLogs();
    const rail = await screen.findByRole('tablist', { name: 'Log files' });
    const detail = screen.getByRole('region', { name: 'Selected log file' });

    // Both panes fill ONE grid, which is what makes them equal height. The height
    // comes from the tab panel, so the grid only has to be a shrinkable flex child.
    const grid = container.querySelector('.grid');
    expect(grid).not.toBeNull();
    expect(grid!.className).toContain('min-h-0');
    expect(grid!.contains(rail)).toBe(true);
    expect(grid!.contains(detail)).toBe(true);
    // Single column below `md`, two columns from `md` up — the same breakpoint
    // and template Health uses, not a new ad-hoc split.
    expect(grid!.className).toContain('md:grid-cols-[var(--rail-w)_minmax(0,1fr)]');

    // The rail is the LEFT pane: it precedes the detail pane in the grid.
    expect(
      grid!.firstElementChild!.contains(rail),
      'the file rail must be the first grid column'
    ).toBe(true);
  });

  it('scrolls the rail internally so it cannot be scrolled out of view', async () => {
    renderLogs();
    const rail = await screen.findByRole('tablist', { name: 'Log files' });
    const detail = screen.getByRole('region', { name: 'Selected log file' });

    // The rail owns a scroll container, so a long file list overflows INSIDE the
    // rail rather than escaping to the tab panel.
    const railScroller = rail.closest('.overflow-y-auto');
    expect(railScroller).not.toBeNull();
    // ...and it is not the detail pane's scroller: neither pane can scroll the
    // other away.
    expect(railScroller!.contains(detail)).toBe(false);

    // The file body is the detail pane's own scroller.
    await waitFor(() => expect(screen.getByText(/beta line/)).toBeInTheDocument());
    const pre = detail.querySelector('pre');
    expect(pre).not.toBeNull();
    expect(pre!.className).toContain('overflow-auto');
    // It takes the pane's leftover height instead of a viewport fraction, which
    // would ignore the pane it actually sits in.
    expect(pre!.className).toContain('min-h-0');
    expect(pre!.className).not.toContain('max-h-[46vh]');
  });

  it('keeps run-level chrome above the split, not inside a pane', async () => {
    // The run picker and the whole-bundle download are about the RUN, so they
    // must not sit inside the rail or the file pane.
    renderLogs();
    const grid = (await screen.findByRole('tablist', { name: 'Log files' })).closest('.grid');
    const download = screen.getByRole('link', { name: /Download full bundle/ });
    expect(grid).not.toBeNull();
    expect(grid!.contains(download)).toBe(false);
  });

  it('exposes the rail as a vertical tablist and roves with Up/Down', async () => {
    const user = userEvent.setup();
    renderLogs();
    const rail = await screen.findByRole('tablist', { name: 'Log files' });
    // A rail is vertical; assistive tech is told so, and the arrow keys that
    // match the axis have to work.
    expect(rail).toHaveAttribute('aria-orientation', 'vertical');

    const driver = screen.getByRole('tab', { name: /driver\.log/ });
    expect(driver).toHaveAttribute('aria-selected', 'true');

    driver.focus();
    await user.keyboard('{ArrowDown}');
    expect(screen.getByRole('tab', { name: /codex\.log/ })).toHaveAttribute(
      'aria-selected',
      'true'
    );
    await user.keyboard('{ArrowUp}');
    expect(screen.getByRole('tab', { name: /driver\.log/ })).toHaveAttribute(
      'aria-selected',
      'true'
    );
  });

  it('renders the #5765 empty state with no split and no manifest request', async () => {
    // The empty state predates the split and must be untouched by it: a neutral
    // note, no grid, and crucially no manifest request behind it.
    const calls: string[] = [];
    vi.unstubAllGlobals();
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        calls.push(url);
        if (url.endsWith('/runs')) return jsonResponse([]);
        throw new Error(`unexpected fetch: ${url}`);
      })
    );
    const { container } = renderLogs();

    expect(
      await screen.findByText('No logs yet — this session has not written any.')
    ).toBeInTheDocument();
    expect(container.querySelector('.grid')).toBeNull();
    expect(calls.filter((u) => u.includes('/manifest'))).toEqual([]);
  });
});
