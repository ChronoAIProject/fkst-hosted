import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { AuthProvider } from '@/lib/auth/github-auth';
import { Toaster, ToastProvider } from '@/components/ui/toast';
import { ChatProvider, useChat } from './chat-context';
import { sseChatTransport } from './transport';

/** A bare `apiFetch` — the stubbed global `fetch`, with no token handling. The
 *  integration cases assert the STREAM path, not authentication. */
const directApiFetch = (path: string, init?: RequestInit) => fetch(path, init);
import { ChatLauncher } from './chat-launcher';
import { ChatPanel } from './chat-panel';
import { renderChat, scriptedTransport } from './chat-test-kit';

/** Mount the launcher + panel together, the way the shell does. */
function Surface() {
  return (
    <>
      <ChatPanel />
      <ChatLauncher />
    </>
  );
}

/** Expose the context so a test can open the panel without clicking. */
const captured: { current: ReturnType<typeof useChat> | null } = { current: null };
function Probe() {
  captured.current = useChat();
  return null;
}
const chat = () => captured.current!;

describe('ChatPanel', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.sessionStorage.clear();
    captured.current = null;
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('resizes by keyboard, clamps at the bounds, and persists the width', () => {
    // Drag-only would put resizing out of reach of keyboard users entirely.
    renderChat(
      <>
        <Probe />
        <Surface />
      </>,
      { transport: scriptedTransport().transport }
    );
    act(() => chat().openPanel());

    const handle = screen.getByTestId('chat-resize');
    const before = Number(handle.getAttribute('aria-valuenow'));
    fireEvent.keyDown(handle, { key: 'ArrowLeft' });
    expect(Number(handle.getAttribute('aria-valuenow'))).toBeGreaterThan(before);

    // Narrow repeatedly: it must settle at the minimum, never below.
    for (let i = 0; i < 40; i += 1) fireEvent.keyDown(handle, { key: 'ArrowRight' });
    const floor = Number(handle.getAttribute('aria-valuenow'));
    expect(floor).toBe(320);
    expect(window.localStorage.getItem('fkst-chat-width')).toBe('320');
  });

  it('toggles full screen and lets Escape leave it WITHOUT closing the panel', () => {
    renderChat(
      <>
        <Probe />
        <Surface />
      </>,
      { transport: scriptedTransport().transport }
    );
    act(() => chat().openPanel());

    const full = screen.getByTestId('chat-fullscreen');
    expect(full).toHaveAttribute('aria-pressed', 'false');
    fireEvent.click(full);
    expect(screen.getByTestId('chat-fullscreen')).toHaveAttribute('aria-pressed', 'true');
    // Full screen has no width to drag, so the handle stands down.
    expect(screen.queryByTestId('chat-resize')).not.toBeInTheDocument();

    fireEvent.keyDown(screen.getByTestId('chat-panel'), { key: 'Escape' });
    expect(screen.getByTestId('chat-fullscreen')).toHaveAttribute('aria-pressed', 'false');
    // Escape peeled one layer only.
    expect(chat().open).toBe(true);
  });

  it('a pinned panel ignores Escape but still closes deliberately', () => {
    renderChat(
      <>
        <Probe />
        <Surface />
      </>,
      { transport: scriptedTransport().transport }
    );
    act(() => chat().openPanel());
    fireEvent.click(screen.getByTestId('chat-pin'));
    expect(screen.getByTestId('chat-pin')).toHaveAttribute('aria-pressed', 'true');

    fireEvent.keyDown(screen.getByTestId('chat-panel'), { key: 'Escape' });
    expect(chat().open).toBe(true);

    fireEvent.click(screen.getByTestId('chat-close'));
    expect(chat().open).toBe(false);
  });

  it('reopens on mount when it was left pinned', () => {
    // Otherwise the pin survives a reload as a preference that visibly does nothing.
    window.localStorage.setItem('fkst-chat-pinned', 'true');
    renderChat(
      <>
        <Probe />
        <Surface />
      </>,
      { transport: scriptedTransport().transport }
    );
    expect(chat().open).toBe(true);
  });

  it('renders the panel chrome, transcript and composer when open', () => {
    renderChat(
      <>
        <Probe />
        <Surface />
      </>,
      { transport: scriptedTransport().transport }
    );

    // Kept mounted while closed (the DrawerShell contract) but not interactive.
    const panel = screen.getByTestId('chat-panel');
    expect(panel).toHaveAttribute('aria-hidden', 'true');
    expect(panel.className).toContain('invisible');

    act(() => chat().openPanel());
    expect(screen.getByTestId('chat-panel')).toHaveAttribute('aria-hidden', 'false');
    expect(screen.getByText('FKST // ORCHESTRATOR')).toBeInTheDocument();
    expect(screen.getByTestId('chat-transcript')).toBeInTheDocument();
    expect(screen.getByTestId('chat-input')).toBeInTheDocument();
  });

  it('carries the entrance animation class only while open', () => {
    // jsdom does no layout, so the assertion is on the class set — which is what
    // drives the animation — not on a measured position.
    renderChat(
      <>
        <Probe />
        <Surface />
      </>,
      { transport: scriptedTransport().transport }
    );
    expect(screen.getByTestId('chat-panel').className).not.toContain('anim-chat-open');
    act(() => chat().openPanel());
    expect(screen.getByTestId('chat-panel').className).toContain('anim-chat-open');
  });

  it('shows the status chip text, not colour alone', () => {
    const script = scriptedTransport();
    renderChat(
      <>
        <Probe />
        <Surface />
      </>,
      { transport: script.transport }
    );
    act(() => chat().openPanel());
    expect(screen.getByText('LINK ACTIVE')).toBeInTheDocument();

    act(() => chat().sendMessage('hi'));
    expect(screen.getByText('STREAMING')).toBeInTheDocument();
    expect(screen.queryByText('LINK ACTIVE')).not.toBeInTheDocument();
  });

  it('closes on the close button', () => {
    renderChat(
      <>
        <Probe />
        <Surface />
      </>,
      { transport: scriptedTransport().transport }
    );
    act(() => chat().openPanel());
    fireEvent.click(screen.getByTestId('chat-close'));
    expect(screen.getByTestId('chat-panel')).toHaveAttribute('aria-hidden', 'true');
  });

  it('closes on Escape pressed inside the panel', () => {
    renderChat(
      <>
        <Probe />
        <Surface />
      </>,
      { transport: scriptedTransport().transport }
    );
    act(() => chat().openPanel());
    fireEvent.keyDown(screen.getByTestId('chat-input'), { key: 'Escape' });
    expect(screen.getByTestId('chat-panel')).toHaveAttribute('aria-hidden', 'true');
  });

  it('ignores Escape pressed outside the panel', () => {
    // This is a NON-modal surface: swallowing Escape globally would break the
    // dashboard's own walk-up.
    renderChat(
      <>
        <Probe />
        <Surface />
      </>,
      { transport: scriptedTransport().transport }
    );
    act(() => chat().openPanel());
    fireEvent.keyDown(document.body, { key: 'Escape' });
    expect(screen.getByTestId('chat-panel')).toHaveAttribute('aria-hidden', 'false');
  });

  it('offers Clear only once there is something to clear', () => {
    const script = scriptedTransport();
    renderChat(
      <>
        <Probe />
        <Surface />
      </>,
      { transport: script.transport }
    );
    act(() => chat().openPanel());
    expect(screen.queryByTestId('chat-clear')).not.toBeInTheDocument();

    act(() => chat().sendMessage('hi'));
    fireEvent.click(screen.getByTestId('chat-clear'));
    expect(screen.queryByTestId('chat-clear')).not.toBeInTheDocument();
    expect(screen.getByText('Ask about your sessions')).toBeInTheDocument();
  });

  it('shows a sign-in card instead of the composer when signed out', () => {
    renderChat(
      <>
        <Probe />
        <Surface />
      </>,
      { transport: scriptedTransport().transport, signedIn: false }
    );
    act(() => chat().openPanel());
    expect(screen.getByTestId('chat-signin-card')).toBeInTheDocument();
    expect(screen.queryByTestId('chat-input')).not.toBeInTheDocument();
  });

  // ---- launcher -----------------------------------------------------------

  it('renders the launcher in its own container, below the toaster', () => {
    // jsdom cannot measure overlap, so the contract is asserted structurally: the
    // launcher is its own element with a z-index below the toaster's z-[60].
    renderChat(<Surface />, { transport: scriptedTransport().transport });
    const launcher = screen.getByTestId('chat-launcher');
    expect(launcher.className).toContain('z-[55]');
    expect(launcher.className).toContain('fixed');
    expect(launcher).not.toContainElement(screen.getByTestId('chat-panel'));
    expect(screen.getByTestId('chat-panel')).not.toContainElement(launcher);
  });

  it('toggles the panel and reflects it in aria-expanded', () => {
    renderChat(<Surface />, { transport: scriptedTransport().transport });
    const launcher = screen.getByTestId('chat-launcher');
    expect(launcher).toHaveAttribute('aria-expanded', 'false');

    fireEvent.click(launcher);
    expect(launcher).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByTestId('chat-panel')).toHaveAttribute('aria-hidden', 'false');

    fireEvent.click(launcher);
    expect(launcher).toHaveAttribute('aria-expanded', 'false');
  });
});

describe('ChatPanel — real SSE transport', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.sessionStorage.clear();
    // Signed in, or the panel shows its sign-in card instead of the transcript.
    window.localStorage.setItem('fkst-gh-access', 'ghu_x');
    // This spec asserts the individual step rows, which CLEAN collapses to a count.
    window.localStorage.setItem('fkst-chat-view-level', 'verbose');
    captured.current = null;
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  /** A `fetch` serving one SSE turn from a real ReadableStream, so the panel is
   *  driven through the SAME parser production uses. */
  function stubSseFetch(frames: string[]) {
    const encoder = new TextEncoder();
    const fetchMock = vi.fn(async () => ({
      ok: true,
      status: 200,
      headers: { get: () => null },
      body: new ReadableStream<Uint8Array>({
        start(controller) {
          frames.forEach((frame) => controller.enqueue(encoder.encode(frame)));
          controller.close();
        },
      }),
      json: async () => ({}),
    }));
    vi.stubGlobal('fetch', fetchMock);
    return fetchMock;
  }

  const frame = (payload: unknown) => `data: ${JSON.stringify(payload)}\n\n`;

  it('streams a real turn into the transcript, with tool chips and a session card', async () => {
    stubSseFetch([
      frame({ type: 'delta', text: 'Looking' }),
      frame({ type: 'tool_call', id: 't1', name: 'list_repo_sessions', args_preview: '{}' }),
      frame({
        type: 'tool_result',
        id: 't1',
        name: 'list_repo_sessions',
        status: 200,
        truncated: false,
      }),
      frame({ type: 'delta', text: ' — one session is live.' }),
      frame({
        type: 'done',
        finish_reason: 'stop',
        session_refs: [
          {
            owner: 'acme',
            name: 'site',
            session_id: 'sess-1',
            trigger_number: 7,
            title: 'nightly',
            status_label: 'fkst-substrate-active',
          },
        ],
      }),
    ]);

    render(
      <ToastProvider>
        <AuthProvider>
          <MemoryRouter>
            <ChatProvider transport={sseChatTransport(directApiFetch)}>
              <Probe />
              <Surface />
            </ChatProvider>
          </MemoryRouter>
        </AuthProvider>
      </ToastProvider>
    );

    act(() => chat().openPanel());
    await act(async () => {
      chat().sendMessage('what is running on acme/site?');
    });

    // Streamed text landed as one growing answer.
    await waitFor(() =>
      expect(screen.getByTestId('chat-assistant-message')).toHaveTextContent(
        'Looking — one session is live.'
      )
    );
    // Tool activity is visible, with its state as text.
    expect(screen.getByText('repo sessions')).toBeInTheDocument();
    expect(screen.getByText(/OK 200/)).toBeInTheDocument();
    // The card came from the DONE frame's structured refs, not from the prose.
    expect(screen.getByTestId('chat-session-card')).toHaveTextContent('nightly');
    expect(screen.getByTestId('chat-card-dashboard-link')).toHaveAttribute(
      'href',
      '/dashboard?owner=acme&repo=site&session=sess-1'
    );
    // The turn ended, so the composer is usable again.
    expect(screen.getByTestId('chat-send')).toBeInTheDocument();
  });

  it('surfaces a stream error as a warning note and a toast', async () => {
    stubSseFetch([
      frame({ type: 'delta', text: 'partial' }),
      frame({ type: 'error', code: 'deadline_exceeded', message: 'too slow' }),
    ]);

    render(
      <ToastProvider>
        <AuthProvider>
          <MemoryRouter>
            <ChatProvider transport={sseChatTransport(directApiFetch)}>
              <Probe />
              <Surface />
              <Toaster dismissLabel="Dismiss" />
            </ChatProvider>
          </MemoryRouter>
        </AuthProvider>
      </ToastProvider>
    );

    act(() => chat().openPanel());
    await act(async () => {
      chat().sendMessage('something slow');
    });

    await waitFor(() =>
      expect(screen.getByTestId('chat-system-note')).toHaveTextContent(
        'That took too long to answer.'
      )
    );
    // The note explains it in place; the toast makes sure it is noticed even when
    // the transcript is scrolled away.
    expect(screen.getAllByText(/That took too long to answer/).length).toBeGreaterThan(1);
    // The transcript survives — only the turn ended.
    expect(screen.getByTestId('chat-user-message')).toHaveTextContent('something slow');
  });
});
