import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { AuthProvider } from '@/lib/auth/github-auth';
import type { IssueDetail, SessionDetail } from '@/lib/api/types';
import { TabLogs } from './tab-logs';

function jsonResponse(body: unknown, status = 200): Response {
  return { ok: status >= 200 && status < 300, status, json: async () => body } as Response;
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

  it('counts in-file search matches', async () => {
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
    // "alpha" appears twice in the fixture.
    expect(await screen.findByText('2 matches')).toBeInTheDocument();
  });

  it('surfaces a manifest load error', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => jsonResponse(null, 403)));
    renderLogs();
    expect(await screen.findByText('Could not load the session logs.')).toBeInTheDocument();
  });
});
