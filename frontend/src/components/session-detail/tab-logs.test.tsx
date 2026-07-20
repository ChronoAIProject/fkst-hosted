import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { AuthProvider } from '@/lib/auth/github-auth';
import type { IssueDetail, SessionDetail } from '@/lib/api/types';
import { TabLogs } from './tab-logs';

function jsonResponse(body: unknown, status = 200): Response {
  return { ok: status >= 200 && status < 300, status, json: async () => body } as Response;
}

/** Parse the `path` / `tail_bytes` query of a stubbed file-fetch URL. */
function fileQuery(url: string): URLSearchParams {
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
  work_label: null,
  auto_merge: null,
  environment: null,
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
        if (url.endsWith('/manifest')) return jsonResponse(manifest);
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
        if (url.endsWith('/manifest')) return jsonResponse(manifest);
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
        if (url.endsWith('/manifest')) {
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
        if (url.endsWith('/manifest')) return jsonResponse(null, 503);
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
        if (url.endsWith('/manifest')) return jsonResponse(manifest);
        if (url.includes('/file?')) {
          const path = fileQuery(url).get('path');
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
        if (url.endsWith('/manifest')) return jsonResponse(manifest);
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
        if (url.endsWith('/manifest')) return jsonResponse(manifest);
        if (url.includes('/file?')) {
          // A tail request returns the truncated view; a full request (no
          // tail_bytes) returns the whole file.
          return fileQuery(url).has('tail_bytes')
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
});
