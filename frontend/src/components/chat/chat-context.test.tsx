import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { act, render } from '@testing-library/react';
import { AuthProvider, useAuth } from '@/lib/auth/github-auth';
import { ToastProvider } from '@/components/ui/toast';
import { ChatProvider, useChat } from './chat-context';
import type { ChatMessage } from './chat-context';
import { renderChat, scriptedTransport } from './chat-test-kit';

const STORAGE_KEY = 'fkst-chat-transcript';

/** A probe exposing the context to the test. */
type Chat = ReturnType<typeof useChat>;
const captured: { current: Chat | null } = { current: null };
function Probe() {
  captured.current = useChat();
  return <span data-testid="probe">{captured.current.messages.length}</span>;
}
const chat = () => captured.current!;

/** The stored transcript as the provider wrote it. */
const stored = (): ChatMessage[] => JSON.parse(window.sessionStorage.getItem(STORAGE_KEY) ?? '[]');

describe('ChatProvider', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.sessionStorage.clear();
    captured.current = null;
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('appends the user message and a growing assistant message', () => {
    const script = scriptedTransport();
    renderChat(<Probe />, { transport: script.transport });

    act(() => chat().sendMessage('what is running?'));
    expect(chat().messages.map((m) => m.role)).toEqual(['user', 'assistant']);
    expect(chat().messages[0]!.content).toBe('what is running?');
    expect(chat().messages[1]!.pending).toBe(true);
    expect(chat().streaming).toBe(true);

    act(() => script.handlers().onDelta('Two '));
    act(() => script.handlers().onDelta('sessions.'));
    expect(chat().messages[1]!.content).toBe('Two sessions.');

    act(() => script.handlers().onDone({ finishReason: 'stop', sessionRefs: [] }));
    expect(chat().messages[1]!.pending).toBe(false);
    expect(chat().streaming).toBe(false);
  });

  it('types a delta out progressively instead of showing it in one chunk', () => {
    // The product rule: a provider that flushes a whole paragraph must still READ as
    // typing. This is the one test that opts out of the kit's reduced-motion default,
    // because the animation is the thing under test.
    vi.useFakeTimers();
    try {
      const script = scriptedTransport();
      renderChat(<Probe />, { transport: script.transport, reducedMotion: false });
      act(() => chat().sendMessage('what is running?'));

      const paragraph = 'Two sessions are running, and both are healthy right now.';
      act(() => script.handlers().onDelta(paragraph));

      // Nothing lands synchronously...
      expect(chat().messages[1]!.content).toBe('');
      // ...and a partial reveal is genuinely partial.
      act(() => vi.advanceTimersByTime(64));
      const partial = chat().messages[1]!.content;
      expect(partial.length).toBeGreaterThan(0);
      expect(partial.length).toBeLessThan(paragraph.length);
      expect(paragraph.startsWith(partial)).toBe(true);

      // The turn stays "streaming" until the reveal drains, so the caret and the
      // disabled composer match what the reader is watching.
      act(() => script.handlers().onDone({ finishReason: 'stop', sessionRefs: [] }));
      expect(chat().streaming).toBe(true);
      expect(chat().messages[1]!.pending).toBe(true);

      act(() => vi.advanceTimersByTime(3000));
      expect(chat().messages[1]!.content).toBe(paragraph);
      expect(chat().messages[1]!.pending).toBe(false);
      expect(chat().streaming).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });

  it('stopping mid-reveal shows what already arrived rather than discarding it', () => {
    vi.useFakeTimers();
    try {
      const script = scriptedTransport();
      renderChat(<Probe />, { transport: script.transport, reducedMotion: false });
      act(() => chat().sendMessage('what is running?'));
      act(() => script.handlers().onDelta('a partially revealed answer'));
      act(() => vi.advanceTimersByTime(32));

      act(() => chat().stopStreaming());
      // Stop means "stop the answer", not "throw away the words already paid for".
      expect(chat().messages[1]!.content).toBe('a partially revealed answer');
      expect(chat().messages[1]!.pending).toBe(false);
      expect(chat().streaming).toBe(false);
      expect(script.aborted()).toBe(true);
    } finally {
      vi.useRealTimers();
    }
  });

  it('attaches structured data cards to the assistant message', () => {
    const script = scriptedTransport();
    renderChat(<Probe />, { transport: script.transport });
    act(() => chat().sendMessage('what environments do I have?'));

    act(() =>
      script.handlers().onDataCard({
        kind: 'environments',
        profiles: [{ name: 'video-studio', status: 'ready' }],
        omitted: 0,
      })
    );
    act(() => script.handlers().onDone({ finishReason: 'stop', sessionRefs: [] }));

    const cards = chat().messages[1]!.dataCards ?? [];
    expect(cards).toHaveLength(1);
    expect(cards[0]).toMatchObject({ kind: 'environments' });
  });

  it('drops an unreadable data card silently rather than noting it', () => {
    // The prose answer still stands on its own; a note about a rendering detail
    // would be noise in the thread.
    const script = scriptedTransport();
    renderChat(<Probe />, { transport: script.transport });
    act(() => chat().sendMessage('hi'));
    act(() => script.handlers().onDataCard({ kind: 'not_a_real_card' }));
    act(() => script.handlers().onDone({ finishReason: 'stop', sessionRefs: [] }));

    expect(chat().messages[1]!.dataCards ?? []).toHaveLength(0);
    expect(chat().messages.filter((m) => m.role === 'system-note')).toHaveLength(0);
  });

  it('sends only user and assistant content on the wire', () => {
    const script = scriptedTransport();
    renderChat(<Probe />, { transport: script.transport });

    act(() => chat().sendMessage('first'));
    act(() => script.handlers().onDelta('answer'));
    act(() => script.handlers().onDone({ finishReason: 'stop', sessionRefs: [] }));
    // An error adds a local system-note, which must never be sent.
    act(() => chat().sendMessage('second'));
    act(() => script.handlers().onError({ code: 'llm_error', message: 'provider down' }));
    act(() => chat().sendMessage('third'));

    const last = script.sent[script.sent.length - 1]!;
    expect(last.map((m) => m.content)).toEqual(['first', 'answer', 'second', 'third']);
    expect(last.every((m) => m.role === 'user' || m.role === 'assistant')).toBe(true);
    // The empty assistant placeholder is never sent either.
    expect(last.some((m) => m.content === '')).toBe(false);
  });

  it('records tool activity on the assistant message', () => {
    const script = scriptedTransport();
    renderChat(<Probe />, { transport: script.transport });

    act(() => chat().sendMessage('why did it fail?'));
    act(() => script.handlers().onToolCall({ id: 't1', name: 'tail_log_file' }));
    expect(chat().messages[1]!.toolEvents).toEqual([{ id: 't1', name: 'tail_log_file' }]);

    act(() =>
      script
        .handlers()
        .onToolResult({ id: 't1', name: 'tail_log_file', status: 403, truncated: false })
    );
    expect(chat().messages[1]!.toolEvents).toEqual([
      { id: 't1', name: 'tail_log_file', status: 403, truncated: false },
    ]);
  });

  it('shows a result with no matching call rather than dropping it', () => {
    // Silence would hide real activity from the user.
    const script = scriptedTransport();
    renderChat(<Probe />, { transport: script.transport });
    act(() => chat().sendMessage('hi'));
    act(() =>
      script
        .handlers()
        .onToolResult({ id: 'orphan', name: 'get_overview', status: 200, truncated: false })
    );
    expect(chat().messages[1]!.toolEvents).toHaveLength(1);
  });

  it('stores the session refs from done onto the finishing message', () => {
    const script = scriptedTransport();
    renderChat(<Probe />, { transport: script.transport });
    act(() => chat().sendMessage('what is running?'));
    const refs = [{ owner: 'acme', name: 'site', trigger_number: 7 }];
    act(() => script.handlers().onDone({ finishReason: 'stop', sessionRefs: refs }));
    expect(chat().messages[1]!.sessionRefs).toEqual(refs);
  });

  it('turns a transport error into a localized warning note', () => {
    // The user-facing string comes from the stable CODE, not the server's prose:
    // the prose is for the log, and a user-facing string must be translatable.
    const script = scriptedTransport();
    renderChat(<Probe />, { transport: script.transport });
    act(() => chat().sendMessage('hi'));
    act(() => script.handlers().onError({ code: 'llm_error', message: 'raw provider text' }));

    const last = chat().messages[chat().messages.length - 1]!;
    expect(last.role).toBe('system-note');
    expect(last.tone).toBe('warn');
    expect(last.content).toBe('The language model could not be reached. Please try again.');
    expect(last.content).not.toContain('raw provider text');
    // The transcript survives; only the turn ended.
    expect(chat().messages.some((m) => m.role === 'user')).toBe(true);
    expect(chat().streaming).toBe(false);
  });

  it('names the retry delay when the server advertised one', () => {
    // "try again in 5s" is actionable where "try again" is not.
    const script = scriptedTransport();
    renderChat(<Probe />, { transport: script.transport });
    act(() => chat().sendMessage('hi'));
    act(() =>
      script.handlers().onError({ code: 'rate_limited', message: 'busy', retryAfterSeconds: 5 })
    );
    expect(chat().messages[chat().messages.length - 1]!.content).toContain('5s');
  });

  it('falls back to the server message for an unrecognized code', () => {
    // A newer backend code must still say something useful rather than nothing.
    const script = scriptedTransport();
    renderChat(<Probe />, { transport: script.transport });
    act(() => chat().sendMessage('hi'));
    act(() => script.handlers().onError({ code: 'brand_new_code', message: 'a new failure' }));
    expect(chat().messages[chat().messages.length - 1]!.content).toBe('a new failure');
  });

  it('aborts the transport on stopStreaming', () => {
    const script = scriptedTransport();
    renderChat(<Probe />, { transport: script.transport });
    act(() => chat().sendMessage('hi'));
    act(() => chat().stopStreaming());

    expect(script.aborted()).toBe(true);
    expect(chat().streaming).toBe(false);
    // No message is left showing a caret that will never resolve.
    expect(chat().messages.every((m) => !m.pending)).toBe(true);
  });

  it('refuses a second turn while one is streaming', () => {
    const script = scriptedTransport();
    renderChat(<Probe />, { transport: script.transport });
    act(() => chat().sendMessage('first'));
    act(() => chat().sendMessage('second'));
    expect(script.sent).toHaveLength(1);
  });

  it('ignores a blank message', () => {
    const script = scriptedTransport();
    renderChat(<Probe />, { transport: script.transport });
    act(() => chat().sendMessage('   '));
    expect(script.sent).toHaveLength(0);
    expect(chat().messages).toHaveLength(0);
  });

  it('clears the transcript and its storage', () => {
    const script = scriptedTransport();
    renderChat(<Probe />, { transport: script.transport });
    act(() => chat().sendMessage('hi'));
    act(() => script.handlers().onDone({ finishReason: 'stop', sessionRefs: [] }));
    expect(stored().length).toBeGreaterThan(0);

    act(() => chat().clearTranscript());
    expect(chat().messages).toHaveLength(0);
    expect(window.sessionStorage.getItem(STORAGE_KEY)).toBeNull();
  });

  // ---- persistence --------------------------------------------------------

  it('round-trips the transcript through sessionStorage', () => {
    const script = scriptedTransport();
    const first = renderChat(<Probe />, { transport: script.transport });
    act(() => chat().sendMessage('remember me'));
    act(() => script.handlers().onDelta('sure'));
    act(() => script.handlers().onDone({ finishReason: 'stop', sessionRefs: [] }));
    first.unmount();

    renderChat(<Probe />, { transport: scriptedTransport().transport });
    expect(chat().messages.map((m) => m.content)).toEqual(['remember me', 'sure']);
  });

  it('never restores a pending caret', () => {
    // A restored `pending` flag would show a caret waiting on a turn that is gone.
    const script = scriptedTransport();
    const first = renderChat(<Probe />, { transport: script.transport });
    act(() => chat().sendMessage('mid-flight'));
    act(() => script.handlers().onDelta('half an ans'));
    first.unmount();

    renderChat(<Probe />, { transport: scriptedTransport().transport });
    expect(chat().messages.every((m) => !m.pending)).toBe(true);
  });

  it('caps the stored transcript', () => {
    const overflowing: ChatMessage[] = Array.from({ length: 130 }, (_, index) => ({
      id: `m-${index}`,
      role: 'user' as const,
      content: `message ${index}`,
    }));
    window.sessionStorage.setItem(STORAGE_KEY, JSON.stringify(overflowing));

    const script = scriptedTransport();
    renderChat(<Probe />, { transport: script.transport });
    // Restored in full, then trimmed on the next write.
    act(() => chat().sendMessage('one more'));
    expect(stored()).toHaveLength(100);
    // The OLDEST go first, so the newest survive: the new message is stored and
    // the earliest restored ones are gone.
    const contents = stored().map((message) => message.content);
    expect(contents).toContain('one more');
    expect(contents).not.toContain('message 0');
    expect(contents).toContain('message 129');
  });

  it('survives a corrupt stored value', () => {
    window.sessionStorage.setItem(STORAGE_KEY, '{not json');
    renderChat(<Probe />, { transport: scriptedTransport().transport });
    expect(chat().messages).toHaveLength(0);
  });

  it('ignores a stored value that is not a message array', () => {
    window.sessionStorage.setItem(STORAGE_KEY, '{"messages":"nope"}');
    renderChat(<Probe />, { transport: scriptedTransport().transport });
    expect(chat().messages).toHaveLength(0);
  });

  // ---- sign-out -----------------------------------------------------------

  it('clears the transcript when the user signs out', () => {
    // On a shared machine one user's conversation must not survive into the next
    // person's session. Driven through the REAL signOut, not by poking state.
    const script = scriptedTransport();
    window.localStorage.setItem('fkst-gh-access', 'ghu_x');
    const auth: { current: ReturnType<typeof useAuth> | null } = { current: null };
    function AuthProbe() {
      auth.current = useAuth();
      return <Probe />;
    }
    render(
      <ToastProvider>
        <AuthProvider>
          <ChatProvider transport={script.transport}>
            <AuthProbe />
          </ChatProvider>
        </AuthProvider>
      </ToastProvider>
    );

    act(() => chat().sendMessage('private question'));
    act(() => script.handlers().onDone({ finishReason: 'stop', sessionRefs: [] }));
    expect(chat().messages.length).toBeGreaterThan(0);

    act(() => auth.current!.signOut());

    expect(chat().messages).toHaveLength(0);
    expect(window.sessionStorage.getItem(STORAGE_KEY)).toBeNull();
  });

  it('keeps a never-signed-in visitor transcript', () => {
    // Only a true->false transition clears; a visitor reading the docs pages
    // signed out must not lose what they were reading.
    const script = scriptedTransport();
    renderChat(<Probe />, { transport: script.transport, signedIn: false });
    act(() => chat().sendMessage('how does this work?'));
    act(() => script.handlers().onDone({ finishReason: 'stop', sessionRefs: [] }));
    expect(chat().messages.length).toBeGreaterThan(0);
  });

  // ---- action proposals ---------------------------------------------------

  it('lands a valid proposal on the assistant message with an id', () => {
    const script = scriptedTransport();
    renderChat(<Probe />, { transport: script.transport });
    act(() => chat().sendMessage('stop my session'));
    act(() =>
      script.handlers().onActionProposal({
        kind: 'stop_session',
        owner: 'acme',
        name: 'site',
        trigger_issue_number: 7,
        reason: 'done',
        summary: 'Stop it',
        target: { method: 'DELETE', path: '/api/v1/repos/acme/site/sessions/7' },
      })
    );

    const proposals = chat().messages[1]!.proposals!;
    expect(proposals).toHaveLength(1);
    expect(proposals[0]!.id).toBeTruthy();
    expect(proposals[0]!.state).toBe('idle');
    expect(proposals[0]!.proposal.kind).toBe('stop_session');
  });

  it('turns an unreadable proposal into a note without killing the stream', () => {
    // The turn is fine; only this draft is not. A card the SPA cannot execute would
    // be worse than saying so.
    const script = scriptedTransport();
    renderChat(<Probe />, { transport: script.transport });
    act(() => chat().sendMessage('do something'));
    act(() => script.handlers().onActionProposal({ kind: 'delete_the_repo' }));

    const last = chat().messages[chat().messages.length - 1]!;
    expect(last.role).toBe('system-note');
    expect(last.content).toContain('unreadable action draft');
    // Not a warning, and not an error toast: nothing went wrong with the turn.
    expect(last.tone).toBe('info');
    expect(chat().messages[1]!.proposals ?? []).toHaveLength(0);

    // The stream continues.
    act(() => script.handlers().onDelta('carrying on'));
    expect(chat().messages[1]!.content).toBe('carrying on');
  });

  it('persists a proposal with its final state', () => {
    const script = scriptedTransport();
    const first = renderChat(<Probe />, { transport: script.transport });
    act(() => chat().sendMessage('stop it'));
    act(() =>
      script.handlers().onActionProposal({
        kind: 'stop_session',
        owner: 'acme',
        name: 'site',
        trigger_issue_number: 7,
        reason: 'done',
        summary: 'Stop it',
        target: { method: 'DELETE', path: '/api/v1/repos/acme/site/sessions/7' },
      })
    );
    act(() => script.handlers().onDone({ finishReason: 'stop', sessionRefs: [] }));
    first.unmount();

    renderChat(<Probe />, { transport: scriptedTransport().transport });
    const restored = chat().messages.find((m) => m.proposals != null)!;
    expect(restored.proposals).toHaveLength(1);
    expect(restored.proposals![0]!.state).toBe('idle');
  });

  it('rehydrates a mid-flight proposal as failed rather than re-running it', () => {
    // The outcome is genuinely unknowable after a reload, and the one thing that
    // must never happen is executing it again silently.
    const stored = [
      {
        id: 'a-1',
        role: 'assistant',
        content: 'here you go',
        proposals: [
          {
            id: 'p-1',
            state: 'executing',
            proposal: {
              kind: 'stop_session',
              owner: 'acme',
              name: 'site',
              trigger_issue_number: 7,
              reason: 'done',
              summary: 'Stop it',
              target: { method: 'DELETE', path: '/api/v1/repos/acme/site/sessions/7' },
            },
          },
        ],
      },
    ];
    window.sessionStorage.setItem(STORAGE_KEY, JSON.stringify(stored));

    renderChat(<Probe />, { transport: scriptedTransport().transport });
    const entry = chat().messages[0]!.proposals![0]!;
    expect(entry.state).toBe('failed');
    expect(entry.error).toBe('restored-unknown');
  });

  it('dismisses a proposal without touching the transcript', () => {
    const script = scriptedTransport();
    renderChat(<Probe />, { transport: script.transport });
    act(() => chat().sendMessage('stop it'));
    act(() =>
      script.handlers().onActionProposal({
        kind: 'stop_session',
        owner: 'acme',
        name: 'site',
        trigger_issue_number: 7,
        reason: 'done',
        summary: 'Stop it',
        target: { method: 'DELETE', path: '/api/v1/repos/acme/site/sessions/7' },
      })
    );
    const id = chat().messages[1]!.proposals![0]!.id;
    act(() => chat().dismissProposal(id));

    expect(chat().messages[1]!.proposals).toHaveLength(0);
    expect(chat().messages[0]!.content).toBe('stop it');
  });

  it('ignores an execute for an unknown id', () => {
    const script = scriptedTransport();
    renderChat(<Probe />, { transport: script.transport });
    // A no-op rather than a throw: a stale card id must not crash the panel.
    return expect(chat().executeProposal('no-such-proposal')).resolves.toBeUndefined();
  });

  it('throws outside a provider rather than silently doing nothing', () => {
    // A mis-mounted panel should fail loudly, not look merely broken.
    const spy = vi.spyOn(console, 'error').mockImplementation(() => {});
    expect(() => render(<Probe />)).toThrow(/ChatProvider/);
    spy.mockRestore();
  });
});
