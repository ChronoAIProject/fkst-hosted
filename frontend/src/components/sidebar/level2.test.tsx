import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter } from 'react-router-dom';
import { AuthProvider } from '@/lib/auth/github-auth';
import { ToastProvider } from '@/components/ui/toast';
import type { IssueDetail, RepoSessionsResponse, SessionDetail } from '@/lib/api/types';
import { buildCreateRequest } from '@/components/modals/create-trigger-modal';
import { Level2Sidebar } from './level2';

function jsonResponse(body: unknown, status = 200): Response {
  return { ok: status >= 200 && status < 300, status, json: async () => body } as Response;
}

const issue = (over: Partial<IssueDetail> & Pick<IssueDetail, 'number' | 'title'>): IssueDetail => ({
  state: 'open',
  author: 'shining',
  labels: [],
  html_url: `https://github.com/shining/lab/issues/${over.number}`,
  created_at: '2026-07-01T02:00:00Z',
  updated_at: '2026-07-02T03:00:00Z',
  closed_at: null,
  ...over,
});

const session = (over: Partial<SessionDetail>): SessionDetail => ({
  session_id: 'f00dfeed-1111-2222-3333-444455556666',
  name: 'nightly',
  work_label: 'fkst-work',
  auto_merge: true,
  environment: 'staging',
  packages: ['ChronoAIProject/fkst-packages@fkst-hosted:codex/base'],
  invalid_reason: null,
  status_labels: ['fkst-substrate-active'],
  trigger: issue({ number: 7, title: 'nightly session' }),
  work_issues: [issue({ number: 9, title: 'implement the thing', state: 'closed' })],
  log_url: 'https://api.example.test/api/v1/logs/f00dfeed-1111-2222-3333-444455556666',
  liveness: 'live',
  prs: [
    {
      number: 12,
      title: 'feat: the thing',
      html_url: 'https://github.com/shining/lab/pull/12',
      state: 'closed',
      merged: true,
      work_issue: 9,
    },
  ],
  ...over,
});

const body = (sessions: SessionDetail[], installed = true): RepoSessionsResponse => ({
  owner: 'shining',
  name: 'lab',
  installed,
  sessions,
});

describe('buildCreateRequest', () => {
  it('trims fields and omits blank optionals entirely', () => {
    expect(
      buildCreateRequest({
        name: '  sess  ',
        packages: [' o/p@main:a ', '', '   '],
        workLabel: '  ',
        environment: '',
        autoMerge: false,
        logAccess: '  ',
        outputLang: '   ',
      })
    ).toEqual({ name: 'sess', packages: ['o/p@main:a'] });
  });

  it('carries the optional knobs when set, splitting the allowlist', () => {
    expect(
      buildCreateRequest({
        name: 'sess',
        packages: ['o/p@main:a', 'o/p@main:b'],
        workLabel: 'lab-work',
        environment: 'staging',
        autoMerge: true,
        logAccess: '@alice, bob  77',
        outputLang: ' 中文 ',
      })
    ).toEqual({
      name: 'sess',
      packages: ['o/p@main:a', 'o/p@main:b'],
      work_label: 'lab-work',
      environment: 'staging',
      auto_merge: true,
      log_access: ['@alice', 'bob', '77'],
      output_lang: '中文',
    });
  });
});

function renderLevel2(props: Partial<Parameters<typeof Level2Sidebar>[0]> = {}) {
  return render(
    <MemoryRouter>
      <ToastProvider>
        <AuthProvider>
          <Level2Sidebar
            owner="shining"
            name="lab"
            data={body([session({})])}
            loadFailed={false}
            onChanged={() => {}}
            {...props}
          />
        </AuthProvider>
      </ToastProvider>
    </MemoryRouter>
  );
}

describe('Level2Sidebar', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.location.hash = '';
    window.localStorage.setItem('fkst-gh-access', 'ghu_x');
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('renders a session card with config metadata, liveness, log link and outcomes', () => {
    renderLevel2();

    // View description + legend are always present and name the repo.
    expect(screen.getByText(/shining\/lab/)).toBeInTheDocument();
    expect(screen.getByText('Legend')).toBeInTheDocument();

    // Session header: name, status label chip, liveness, auto-merge.
    expect(screen.getByText('nightly')).toBeInTheDocument();
    expect(screen.getByText('fkst-substrate-active')).toBeInTheDocument();
    expect(screen.getByText('live')).toBeInTheDocument();
    expect(screen.getByText('auto-merge')).toBeInTheDocument();

    // Config metadata: work label + environment + packages.
    expect(screen.getByText('fkst-work')).toBeInTheDocument();
    expect(screen.getByText('staging')).toBeInTheDocument();
    expect(
      screen.getByText('ChronoAIProject/fkst-packages@fkst-hosted:codex/base')
    ).toBeInTheDocument();

    // Trigger timestamps render as viewer-local relative text with the full
    // absolute value one hover away (title tooltip). The exact relative bucket
    // depends on the wall clock, so assert the label + that a tooltip backs it.
    const created = screen.getByText(/^created\b/);
    expect(created).toHaveAttribute('title');

    // The header freshness line ticks off the last successful poll — seeded at
    // mount, so it reads "updated now" immediately (distinct from the card's
    // own "updated …" trigger timestamp).
    const freshness = screen.getByText('updated now');
    expect(freshness).toHaveAttribute('title', 'Auto-refreshes every 15 s while open.');

    // Log download link goes straight at log_url.
    const log = screen.getByRole('link', { name: /Download logs/ });
    expect(log).toHaveAttribute(
      'href',
      'https://api.example.test/api/v1/logs/f00dfeed-1111-2222-3333-444455556666'
    );

    // Trigger + work issue links.
    expect(screen.getByRole('link', { name: '#7' })).toHaveAttribute(
      'href',
      'https://github.com/shining/lab/issues/7'
    );
    expect(screen.getByRole('link', { name: '#9' })).toHaveAttribute(
      'href',
      'https://github.com/shining/lab/issues/9'
    );

    // PR outcome row: link, merged chip, work-issue backlink.
    expect(screen.getByRole('link', { name: '#12' })).toHaveAttribute(
      'href',
      'https://github.com/shining/lab/pull/12'
    );
    expect(screen.getByText('merged')).toBeInTheDocument();
    expect(screen.getByText('for #9')).toBeInTheDocument();
  });

  it('shows the invalid reason verbatim and the not-installed note', () => {
    renderLevel2({
      data: body(
        [session({ invalid_reason: 'Packages: line 2 is unreachable.', name: null })],
        false
      ),
    });
    expect(screen.getByText('Invalid trigger')).toBeInTheDocument();
    expect(screen.getByText('Packages: line 2 is unreachable.')).toBeInTheDocument();
    expect(
      screen.getByText('The App is not installed on this repository, so sessions cannot run here.')
    ).toBeInTheDocument();
  });

  it('shows empty and failure states', () => {
    renderLevel2({ data: body([]) });
    expect(screen.getByText('No fkst sessions in this repository.')).toBeInTheDocument();

    renderLevel2({ data: null, loadFailed: true });
    expect(
      screen.getByText('Could not load the sessions of this repository. Please try again.')
    ).toBeInTheDocument();
  });

  it('keeps the session list with a stale notice when a refresh fails', () => {
    renderLevel2({ data: body([session({})]), loadFailed: true });

    // Last-good data stays on screen; only the non-blocking notice is added.
    expect(screen.getByText('nightly')).toBeInTheDocument();
    expect(screen.getByText('Refresh failed — showing the last loaded sessions.')).toBeInTheDocument();
    expect(
      screen.queryByText('Could not load the sessions of this repository. Please try again.')
    ).not.toBeInTheDocument();
  });

  it('creates a trigger: POSTs the form, omits blank optionals, notifies', async () => {
    const user = userEvent.setup();
    const onChanged = vi.fn();
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.endsWith('/api/v1/repos/shining/lab/sessions') && init?.method === 'POST') {
        return jsonResponse(
          { issue_number: 30, html_url: 'https://github.com/shining/lab/issues/30' },
          201
        );
      }
      throw new Error(`unexpected fetch: ${url}`);
    });
    vi.stubGlobal('fetch', fetchMock);
    renderLevel2({ onChanged });

    await user.click(screen.getByRole('button', { name: 'New session' }));
    const dialog = await screen.findByRole('dialog');
    const submit = within(dialog).getByRole('button', { name: 'Create trigger issue' });
    expect(submit).toBeDisabled(); // name + >=1 package required

    await user.type(within(dialog).getByLabelText('Session name'), 'workhorse');
    expect(submit).toBeDisabled(); // still no package

    await user.type(within(dialog).getByLabelText('Packages 1'), 'o/p@main:pkg/a');
    // A second package row can be added and removed.
    await user.click(within(dialog).getByRole('button', { name: 'Add package' }));
    await user.type(within(dialog).getByLabelText('Packages 2'), 'o/p@main:pkg/b');
    await user.click(within(dialog).getByRole('button', { name: 'Remove package 2' }));

    await user.type(within(dialog).getByLabelText('Work label (optional)'), 'lab-work');
    await user.click(within(dialog).getByRole('checkbox', { name: 'Auto-merge' }));
    await user.type(within(dialog).getByLabelText('Output language (optional)'), 'English');
    await user.click(submit);

    await waitFor(() => expect(onChanged).toHaveBeenCalled());
    const post = fetchMock.mock.calls.find(([, init]) => init?.method === 'POST')!;
    expect(JSON.parse(String(post[1]!.body))).toEqual({
      name: 'workhorse',
      packages: ['o/p@main:pkg/a'],
      work_label: 'lab-work',
      auto_merge: true,
      output_lang: 'English',
    });
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
  });

  it('surfaces the create 400 message verbatim and keeps the dialog open', async () => {
    const user = userEvent.setup();
    const message = 'Session Name: must be a single DNS-label-ish line.';
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => jsonResponse({ error: 'invalid_trigger', message }, 400))
    );
    renderLevel2();

    await user.click(screen.getByRole('button', { name: 'New session' }));
    const dialog = await screen.findByRole('dialog');
    await user.type(within(dialog).getByLabelText('Session name'), 'Bad Name');
    await user.type(within(dialog).getByLabelText('Packages 1'), 'o/p@main:pkg');
    await user.click(within(dialog).getByRole('button', { name: 'Create trigger issue' }));

    expect(await within(dialog).findByText(message)).toBeInTheDocument();
    expect(screen.getByRole('dialog')).toBeInTheDocument();
  });

  it('stops a session through the confirm dialog (DELETE) and notifies', async () => {
    const user = userEvent.setup();
    const onChanged = vi.fn();
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      if (init?.method === 'DELETE') return jsonResponse(null, 204);
      throw new Error(`unexpected fetch: ${String(input)}`);
    });
    vi.stubGlobal('fetch', fetchMock);
    renderLevel2({ onChanged });

    await user.click(screen.getByRole('button', { name: 'Stop session nightly' }));
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByText('Stop session nightly?')).toBeInTheDocument();
    expect(within(dialog).getByText(/#7/)).toBeInTheDocument();
    await user.click(within(dialog).getByRole('button', { name: 'Stop session' }));

    await waitFor(() => expect(onChanged).toHaveBeenCalled());
    const del = fetchMock.mock.calls.find(([, init]) => init?.method === 'DELETE')!;
    expect(String(del[0])).toMatch(/\/api\/v1\/repos\/shining\/lab\/sessions\/7$/);
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
  });

  it('keeps the stop dialog open with the envelope message on failure', async () => {
    const user = userEvent.setup();
    const message = 'GitHub said no: not authorized to close this issue.';
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => jsonResponse({ error: 'forbidden', message }, 403))
    );
    renderLevel2();

    await user.click(screen.getByRole('button', { name: 'Stop session nightly' }));
    const dialog = await screen.findByRole('dialog');
    await user.click(within(dialog).getByRole('button', { name: 'Stop session' }));

    expect(await within(dialog).findByText(message)).toBeInTheDocument();
    expect(screen.getByRole('dialog')).toBeInTheDocument();
  });

  it('offers no Stop button on a closed trigger', () => {
    renderLevel2({
      data: body([
        session({
          trigger: issue({ number: 7, title: 'done', state: 'closed', closed_at: '2026-07-03T00:00:00Z' }),
          status_labels: [],
          liveness: null,
        }),
      ]),
    });
    expect(screen.queryByRole('button', { name: /Stop session/ })).not.toBeInTheDocument();
    // Closed trigger renders a "closed <relative>" timestamp (a bare "closed"
    // issue-state word also exists, so require text after the label).
    expect(screen.getByText(/^closed\s+\S/)).toBeInTheDocument();
  });

  it('offers a Retry on a no-data load failure and refreshes immediately', async () => {
    const user = userEvent.setup();
    const onChanged = vi.fn();
    const { rerender } = render(
      <AuthProvider>
        <Level2Sidebar owner="shining" name="lab" data={null} loadFailed onChanged={onChanged} />
      </AuthProvider>
    );

    // The dead-end red note now carries an actionable Retry.
    expect(
      screen.getByText('Could not load the sessions of this repository. Please try again.')
    ).toBeInTheDocument();
    const retry = screen.getByRole('button', { name: 'Retry' });
    await user.click(retry);

    // Recovery does not wait for the silent poll: the parent re-fetch fires now.
    expect(onChanged).toHaveBeenCalledTimes(1);
    // In-flight: the button flips to the pending label and is disabled.
    expect(screen.getByRole('button', { name: 'Refreshing…' })).toBeDisabled();

    // The parent resolves the refresh with data → list shows and freshness is
    // stamped, clearing the spinner.
    rerender(
      <AuthProvider>
        <Level2Sidebar
          owner="shining"
          name="lab"
          data={body([session({})])}
          loadFailed={false}
          onChanged={onChanged}
        />
      </AuthProvider>
    );
    expect(screen.getByText('nightly')).toBeInTheDocument();
    expect(screen.getByText('updated now')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Refreshing…' })).not.toBeInTheDocument();
  });

  it('hides the freshness line until a load actually succeeds', () => {
    // Mounting with a stale payload behind a failed refresh: we never observed
    // a success, so no misleading "updated" time is shown — only the notice.
    renderLevel2({ data: body([session({})]), loadFailed: true });
    expect(screen.queryByText('updated now')).not.toBeInTheDocument();
    expect(
      screen.getByText('Refresh failed — showing the last loaded sessions.')
    ).toBeInTheDocument();
  });

  it('keeps a session card mounted across a poll reorder (stable key, B2)', () => {
    // Two sessions; a poll swaps their order. With a positional-index key the
    // cards would remount (slamming an open drawer shut). The key is the stable
    // session_id, so the DOM node for a given session must survive the reorder.
    // Trigger titles are kept distinct from the session names so the name span
    // is the sole element matching 'alpha' (the trigger title also renders).
    const alpha = session({
      session_id: 'aaaaaaaa-0000-0000-0000-000000000000',
      name: 'alpha',
      work_issues: [],
      prs: [],
      trigger: issue({ number: 1, title: 'a-trig' }),
    });
    const beta = session({
      session_id: 'bbbbbbbb-1111-1111-1111-111111111111',
      name: 'beta',
      work_issues: [],
      prs: [],
      trigger: issue({ number: 2, title: 'b-trig' }),
    });

    const { rerender } = render(
      <AuthProvider>
        <Level2Sidebar
          owner="shining"
          name="lab"
          data={body([alpha, beta])}
          loadFailed={false}
          onChanged={() => {}}
        />
      </AuthProvider>
    );
    const alphaNode = screen.getByText('alpha', { selector: 'span' });

    rerender(
      <AuthProvider>
        <Level2Sidebar
          owner="shining"
          name="lab"
          data={body([beta, alpha])}
          loadFailed={false}
          onChanged={() => {}}
        />
      </AuthProvider>
    );
    // Same element instance ⇒ React moved (did not remount) the card.
    expect(screen.getByText('alpha', { selector: 'span' })).toBe(alphaNode);
  });
});
