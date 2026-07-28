import { describe, it, expect, beforeEach, vi } from 'vitest';
import { act, fireEvent, screen } from '@testing-library/react';
import { useChat } from './chat-context';
import { MessageList } from './message-list';
import { renderChat, scriptedTransport } from './chat-test-kit';

const captured: { current: ReturnType<typeof useChat> | null } = { current: null };
function Host({ onPick = () => {} }: { onPick?: (text: string) => void }) {
  captured.current = useChat();
  return <MessageList onPickStarter={onPick} />;
}
const chat = () => captured.current!;

/** Give the transcript a measurable scroll geometry. jsdom reports every size as
 *  0, so the stick-to-bottom rule can only be exercised by defining them. */
function setGeometry(
  element: HTMLElement,
  geometry: { scrollHeight: number; clientHeight: number; scrollTop: number }
) {
  Object.defineProperty(element, 'scrollHeight', {
    configurable: true,
    value: geometry.scrollHeight,
  });
  Object.defineProperty(element, 'clientHeight', {
    configurable: true,
    value: geometry.clientHeight,
  });
  Object.defineProperty(element, 'scrollTop', {
    configurable: true,
    writable: true,
    value: geometry.scrollTop,
  });
}

/** The scrolling element is the ScrollArea, which is the transcript's parent. */
const scroller = () => screen.getByTestId('chat-transcript').parentElement!;

describe('MessageList', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.sessionStorage.clear();
    captured.current = null;
    vi.restoreAllMocks();
  });

  it('shows the welcome card while the transcript is empty', () => {
    renderChat(<Host />, { transport: scriptedTransport().transport });
    expect(screen.getByText('Ask about your sessions')).toBeInTheDocument();
    expect(screen.getByRole('log')).toHaveAttribute('aria-live', 'polite');
  });

  it('prefills the composer from a starter prompt', () => {
    const onPick = vi.fn();
    renderChat(<Host onPick={onPick} />, { transport: scriptedTransport().transport });
    fireEvent.click(screen.getByRole('button', { name: 'What sessions are running?' }));
    expect(onPick).toHaveBeenCalledWith('What sessions are running?');
  });

  it('replaces the welcome card with the transcript once a turn starts', () => {
    const script = scriptedTransport();
    renderChat(<Host />, { transport: script.transport });
    act(() => chat().sendMessage('hi'));

    expect(screen.queryByText('Ask about your sessions')).not.toBeInTheDocument();
    expect(screen.getByTestId('chat-user-message')).toHaveTextContent('hi');
    expect(screen.getByTestId('chat-assistant-message')).toBeInTheDocument();
    // The pending caret IS the typing indicator.
    expect(screen.getByTestId('chat-pending-caret')).toBeInTheDocument();
  });

  it('renders tool activity with its state as TEXT', () => {
    const script = scriptedTransport();
    renderChat(<Host />, { transport: script.transport });
    act(() => chat().sendMessage('why did it fail?'));

    act(() => script.handlers().onToolCall({ id: 't1', name: 'tail_log_file' }));
    // Humanized from the i18n map; the raw wire name is the fallback.
    expect(screen.getByText('log tail')).toBeInTheDocument();
    expect(screen.getByText(/RUNNING/)).toBeInTheDocument();

    act(() =>
      script
        .handlers()
        .onToolResult({ id: 't1', name: 'tail_log_file', status: 403, truncated: false })
    );
    // A denial is stated, not merely coloured — and it reads as DENIED, not ERR,
    // because it is an answer about access.
    expect(screen.getByText(/DENIED 403/)).toBeInTheDocument();
  });

  it('marks a truncated tool result', () => {
    const script = scriptedTransport();
    renderChat(<Host />, { transport: script.transport });
    act(() => chat().sendMessage('read the log'));
    act(() =>
      script
        .handlers()
        .onToolResult({ id: 't1', name: 'tail_log_file', status: 200, truncated: true })
    );
    expect(screen.getByText(/TRUNCATED/)).toBeInTheDocument();
  });

  it('renders an error as a warning note in the same thread', () => {
    const script = scriptedTransport();
    renderChat(<Host />, { transport: script.transport });
    act(() => chat().sendMessage('hi'));
    act(() => script.handlers().onError({ code: 'llm_error', message: 'provider unreachable' }));

    const note = screen.getByTestId('chat-system-note');
    // Localized from the code, not the server's prose.
    expect(note).toHaveTextContent('The language model could not be reached.');
    // Warning tone is --warn, never the brand accent.
    expect(note.className).toContain('text-warn');
  });

  // ---- stick to bottom ----------------------------------------------------

  it('offers "jump to latest" once the user scrolls away from the bottom', () => {
    const script = scriptedTransport();
    renderChat(<Host />, { transport: script.transport });
    act(() => chat().sendMessage('hi'));

    const element = scroller();
    expect(screen.queryByTestId('chat-jump-latest')).not.toBeInTheDocument();

    // Scrolled far from the bottom: the user is reading, so do not drag them down.
    setGeometry(element, { scrollHeight: 1000, clientHeight: 300, scrollTop: 100 });
    fireEvent.scroll(element);
    expect(screen.getByTestId('chat-jump-latest')).toBeInTheDocument();

    fireEvent.click(screen.getByTestId('chat-jump-latest'));
    expect(element.scrollTop).toBe(1000);
    expect(screen.queryByTestId('chat-jump-latest')).not.toBeInTheDocument();
  });

  it('stays attached while the user is near the bottom', () => {
    const script = scriptedTransport();
    renderChat(<Host />, { transport: script.transport });
    act(() => chat().sendMessage('hi'));

    const element = scroller();
    // Within the threshold, so still "following the conversation".
    setGeometry(element, { scrollHeight: 1000, clientHeight: 300, scrollTop: 680 });
    fireEvent.scroll(element);
    expect(screen.queryByTestId('chat-jump-latest')).not.toBeInTheDocument();
  });

  it('follows growing content while attached', () => {
    const script = scriptedTransport();
    renderChat(<Host />, { transport: script.transport });
    act(() => chat().sendMessage('hi'));

    const element = scroller();
    setGeometry(element, { scrollHeight: 1200, clientHeight: 300, scrollTop: 0 });
    act(() => script.handlers().onDelta('a growing answer'));
    // Attached, so the new content is scrolled to.
    expect(element.scrollTop).toBe(1200);
  });
});
