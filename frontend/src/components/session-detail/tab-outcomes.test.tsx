import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { AuthProvider } from '@/lib/auth/github-auth';
import { TabOutcomes } from './tab-outcomes';

function jsonResponse(body: unknown, status = 200): Response {
  return { ok: status >= 200 && status < 300, status, json: async () => body } as Response;
}
function blobResponse(blob: Blob, status = 200): Response {
  return { ok: status >= 200 && status < 300, status, blob: async () => blob } as Response;
}

const outcomesBody = {
  owner: 'shining',
  name: 'lab',
  trigger_issue: 7,
  prs: [
    {
      number: 12,
      title: 'feat: the thing',
      html_url: 'https://github.com/shining/lab/pull/12',
      state: 'closed',
      merged: true,
      work_issue: 9,
      files_error: false,
      files: [
        {
          filename: 'src/a.ts',
          status: 'added',
          additions: 10,
          deletions: 2,
          sha: 'deadbeef',
          previous_filename: null,
          kind: 'text',
          size_hint: 12,
        },
      ],
    },
  ],
};

function renderOutcomes() {
  return render(
    <AuthProvider>
      <TabOutcomes owner="shining" name="lab" issue={7} />
    </AuthProvider>
  );
}

describe('TabOutcomes', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.location.hash = '';
    window.localStorage.setItem('fkst-gh-access', 'ghu_x');
  });
  afterEach(() => vi.unstubAllGlobals());

  it('renders a PR block with its merged chip and file row', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => jsonResponse(outcomesBody)));
    renderOutcomes();

    expect(await screen.findByText('feat: the thing')).toBeInTheDocument();
    expect(screen.getByRole('link', { name: '#12' })).toHaveAttribute(
      'href',
      'https://github.com/shining/lab/pull/12'
    );
    expect(screen.getByText('merged')).toBeInTheDocument();
    expect(screen.getByText('for #9')).toBeInTheDocument();
    expect(screen.getByText('src/a.ts')).toBeInTheDocument();
    expect(screen.getByText('+10')).toBeInTheDocument();
    expect(screen.getByText('-2')).toBeInTheDocument();
    expect(screen.getByText('added')).toBeInTheDocument();
  });

  it('expands a text file into an inline preview', async () => {
    const user = userEvent.setup();
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes('/blob/')) return blobResponse(new Blob(['hello from the file']));
        return jsonResponse(outcomesBody);
      })
    );
    renderOutcomes();

    await user.click(await screen.findByText('src/a.ts'));
    expect(await screen.findByText('hello from the file')).toBeInTheDocument();
  });

  it('shows the empty state when there are no PRs', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => jsonResponse({ ...outcomesBody, prs: [] })));
    renderOutcomes();
    expect(
      await screen.findByText('No pull requests for this session yet.')
    ).toBeInTheDocument();
  });

  it('surfaces a load error', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => jsonResponse(null, 500)));
    renderOutcomes();
    expect(
      await screen.findByText('Could not load the session outcomes.')
    ).toBeInTheDocument();
  });
});
