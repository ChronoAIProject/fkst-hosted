import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { AuthProvider } from '@/lib/auth/github-auth';
import { TabOutcomes } from './tab-outcomes';

function jsonResponse(body: unknown, status = 200): Response {
  return { ok: status >= 200 && status < 300, status, json: async () => body } as Response;
}
function blobResponse(blob: Blob, status = 200): Response {
  return { ok: status >= 200 && status < 300, status, blob: async () => blob } as Response;
}

const textFile = {
  filename: 'src/a.ts',
  status: 'added',
  additions: 10,
  deletions: 2,
  sha: 'deadbeef',
  previous_filename: null,
  kind: 'text',
  size_hint: 12,
};
const binaryFile = {
  filename: 'assets/logo.png',
  status: 'added',
  additions: 0,
  deletions: 0,
  sha: 'cafebabe',
  previous_filename: null,
  kind: 'binary',
  size_hint: null,
};

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
      files: [textFile, binaryFile],
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

/** The button element that toggles a file row (the row's clickable filename). */
function rowButton(filename: string): HTMLButtonElement {
  return screen.getByText(filename).closest('button') as HTMLButtonElement;
}

describe('TabOutcomes', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.location.hash = '';
    window.localStorage.setItem('fkst-gh-access', 'ghu_x');
  });
  afterEach(() => vi.unstubAllGlobals());

  it('renders a PR block with its merged chip, file row and size', async () => {
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
    // size_hint surfaced next to the row for the text file...
    expect(screen.getByText('12 lines')).toBeInTheDocument();
  });

  it('defers the byte fetch behind an explicit Load preview click', async () => {
    const user = userEvent.setup();
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.includes('/blob/')) return blobResponse(new Blob(['hello from the file']));
      return jsonResponse(outcomesBody);
    });
    vi.stubGlobal('fetch', fetchMock);
    renderOutcomes();

    // Expanding the row must NOT fetch the blob — only reveal the Load button.
    await user.click(await screen.findByText('src/a.ts'));
    const loadBtn = await screen.findByText('Load preview');
    expect(fetchMock.mock.calls.some(([u]) => String(u).includes('/blob/'))).toBe(false);

    await user.click(loadBtn);
    expect(await screen.findByText('hello from the file')).toBeInTheDocument();
    expect(fetchMock.mock.calls.some(([u]) => String(u).includes('/blob/'))).toBe(true);
  });

  it('expands rows independently — a second row does not collapse the first', async () => {
    const user = userEvent.setup();
    vi.stubGlobal('fetch', vi.fn(async () => jsonResponse(outcomesBody)));
    renderOutcomes();

    await user.click(await screen.findByText('src/a.ts'));
    expect(rowButton('src/a.ts')).toHaveAttribute('aria-expanded', 'true');

    await user.click(rowButton('assets/logo.png'));
    // Both stay open (the old single-expand silently collapsed the first).
    expect(rowButton('assets/logo.png')).toHaveAttribute('aria-expanded', 'true');
    expect(rowButton('src/a.ts')).toHaveAttribute('aria-expanded', 'true');

    // A binary file needs no fetch — it shows the download-to-view note.
    expect(await screen.findByText('Binary file — download to view.')).toBeInTheDocument();

    // Toggling the first row off collapses only it.
    await user.click(rowButton('src/a.ts'));
    await waitFor(() => expect(rowButton('src/a.ts')).toHaveAttribute('aria-expanded', 'false'));
    expect(rowButton('assets/logo.png')).toHaveAttribute('aria-expanded', 'true');
  });

  it('degrades a 413 preview to an open-on-GitHub affordance', async () => {
    const user = userEvent.setup();
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes('/blob/')) return blobResponse(new Blob([]), 413);
        return jsonResponse(outcomesBody);
      })
    );
    renderOutcomes();

    await user.click(await screen.findByText('src/a.ts'));
    await user.click(await screen.findByText('Load preview'));

    expect(await screen.findByText('This file is too large to preview here.')).toBeInTheDocument();
    expect(screen.getByRole('link', { name: /Open on GitHub/ })).toHaveAttribute(
      'href',
      'https://github.com/shining/lab/pull/12/files'
    );
  });

  it('offers Retry when a preview fetch fails, and recovers on retry', async () => {
    const user = userEvent.setup();
    let blobCalls = 0;
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes('/blob/')) {
          blobCalls += 1;
          return blobCalls === 1 ? blobResponse(new Blob([]), 500) : blobResponse(new Blob(['recovered']));
        }
        return jsonResponse(outcomesBody);
      })
    );
    renderOutcomes();

    await user.click(await screen.findByText('src/a.ts'));
    await user.click(await screen.findByText('Load preview'));
    expect(await screen.findByText('Could not load this file.')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Retry' }));
    expect(await screen.findByText('recovered')).toBeInTheDocument();
  });

  it('revokes a media object URL when the tab unmounts', async () => {
    const user = userEvent.setup();
    // Preserve the URL constructor (auth/other globals may need it); only swap
    // the object-URL statics jsdom omits.
    const createObjectURL = vi.fn(() => 'blob:fake-url');
    const revokeObjectURL = vi.fn();
    const origCreate = URL.createObjectURL;
    const origRevoke = URL.revokeObjectURL;
    URL.createObjectURL = createObjectURL;
    URL.revokeObjectURL = revokeObjectURL;
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes('/blob/')) return blobResponse(new Blob(['png-bytes']));
        return jsonResponse({
          ...outcomesBody,
          prs: [{ ...outcomesBody.prs[0], files: [{ ...binaryFile, kind: 'image' }] }],
        });
      })
    );
    const { unmount } = renderOutcomes();

    await user.click(await screen.findByText('assets/logo.png'));
    await user.click(await screen.findByText('Load preview'));
    await waitFor(() => expect(createObjectURL).toHaveBeenCalledTimes(1));

    unmount();
    expect(revokeObjectURL).toHaveBeenCalledWith('blob:fake-url');

    URL.createObjectURL = origCreate;
    URL.revokeObjectURL = origRevoke;
  });

  it('shows the empty state when there are no PRs', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => jsonResponse({ ...outcomesBody, prs: [] })));
    renderOutcomes();
    expect(await screen.findByText('No pull requests for this session yet.')).toBeInTheDocument();
  });

  it('surfaces a load error and retries the outcomes fetch', async () => {
    const user = userEvent.setup();
    let calls = 0;
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => {
        calls += 1;
        return calls === 1 ? jsonResponse(null, 500) : jsonResponse(outcomesBody);
      })
    );
    renderOutcomes();

    expect(await screen.findByText('Could not load the session outcomes.')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Retry' }));
    expect(await screen.findByText('feat: the thing')).toBeInTheDocument();
  });
});
