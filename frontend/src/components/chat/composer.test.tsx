import { describe, it, expect, beforeEach, vi } from 'vitest';
import { act, fireEvent, screen } from '@testing-library/react';
import { useState } from 'react';
import { Composer, MAX_MESSAGE_CHARS } from './composer';
import { useChat } from './chat-context';
import { renderChat, scriptedTransport } from './chat-test-kit';

/** A host owning the draft, exactly as ChatPanel does. */
const captured: { current: ReturnType<typeof useChat> | null } = { current: null };
function Host({ initial = '' }: { initial?: string }) {
  const [draft, setDraft] = useState(initial);
  captured.current = useChat();
  return <Composer value={draft} onChange={setDraft} />;
}
const chat = () => captured.current!;

const input = () => screen.getByTestId('chat-input') as HTMLTextAreaElement;

describe('Composer', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.sessionStorage.clear();
    captured.current = null;
    vi.restoreAllMocks();
  });

  it('sends on Enter', () => {
    const script = scriptedTransport();
    renderChat(<Host />, { transport: script.transport });
    fireEvent.change(input(), { target: { value: 'what is running?' } });
    fireEvent.keyDown(input(), { key: 'Enter' });

    expect(script.sent).toHaveLength(1);
    expect(script.sent[0]![0]!.content).toBe('what is running?');
    // The field clears so the next question starts fresh.
    expect(input().value).toBe('');
  });

  it('inserts a newline on Shift+Enter instead of sending', () => {
    const script = scriptedTransport();
    renderChat(<Host />, { transport: script.transport });
    fireEvent.change(input(), { target: { value: 'line one' } });
    fireEvent.keyDown(input(), { key: 'Enter', shiftKey: true });

    expect(script.sent).toHaveLength(0);
    // The default action is left alone, so the browser adds the newline.
    expect(input().value).toBe('line one');
  });

  it('sends on the send button', () => {
    const script = scriptedTransport();
    renderChat(<Host />, { transport: script.transport });
    fireEvent.change(input(), { target: { value: 'hello' } });
    fireEvent.click(screen.getByTestId('chat-send'));
    expect(script.sent).toHaveLength(1);
  });

  it('disables send while the field is empty or whitespace', () => {
    renderChat(<Host />, { transport: scriptedTransport().transport });
    expect(screen.getByTestId('chat-send')).toBeDisabled();

    fireEvent.change(input(), { target: { value: '   ' } });
    expect(screen.getByTestId('chat-send')).toBeDisabled();

    fireEvent.change(input(), { target: { value: 'x' } });
    expect(screen.getByTestId('chat-send')).toBeEnabled();
  });

  it('ignores Enter on an empty field', () => {
    const script = scriptedTransport();
    renderChat(<Host />, { transport: script.transport });
    fireEvent.keyDown(input(), { key: 'Enter' });
    expect(script.sent).toHaveLength(0);
  });

  it('offers Stop ALONGSIDE Send while streaming, and stops the turn', () => {
    // Stop used to REPLACE Send, which is what forced the two-step interrupt.
    // They now coexist because they do different things: Stop discards the
    // answer, Send interrupts it and asks the next question (#5620).
    const script = scriptedTransport();
    renderChat(<Host />, { transport: script.transport });
    fireEvent.change(input(), { target: { value: 'hi' } });
    fireEvent.keyDown(input(), { key: 'Enter' });

    expect(screen.getByTestId('chat-send')).toBeInTheDocument();
    const stop = screen.getByTestId('chat-stop');
    // Stopping is a caution, so it carries --warn — never the brand accent.
    expect(stop.className).toContain('text-warn');

    fireEvent.click(stop);
    expect(script.aborted()).toBe(true);
    // Stop leaves once the turn is over; Send remains the primary action.
    expect(screen.queryByTestId('chat-stop')).not.toBeInTheDocument();
    expect(screen.getByTestId('chat-send')).toBeInTheDocument();
  });

  it('sends a second message while streaming, interrupting the first', () => {
    const script = scriptedTransport();
    renderChat(<Host />, { transport: script.transport });
    fireEvent.change(input(), { target: { value: 'first' } });
    fireEvent.keyDown(input(), { key: 'Enter' });
    fireEvent.change(input(), { target: { value: 'second' } });
    fireEvent.keyDown(input(), { key: 'Enter' });
    // No intermediate Stop press: the second question goes out immediately.
    expect(script.sent).toHaveLength(2);
    expect(script.aborted()).toBe(true);
  });

  it('caps the message length', () => {
    renderChat(<Host />, { transport: scriptedTransport().transport });
    fireEvent.change(input(), { target: { value: 'x'.repeat(MAX_MESSAGE_CHARS + 500) } });
    expect(input().value).toHaveLength(MAX_MESSAGE_CHARS);
  });

  it('shows the counter only near the cap', () => {
    renderChat(<Host />, { transport: scriptedTransport().transport });
    fireEvent.change(input(), { target: { value: 'short' } });
    expect(screen.queryByTestId('chat-char-count')).not.toBeInTheDocument();

    fireEvent.change(input(), { target: { value: 'x'.repeat(MAX_MESSAGE_CHARS) } });
    const counter = screen.getByTestId('chat-char-count');
    expect(counter).toHaveTextContent(`${MAX_MESSAGE_CHARS} / ${MAX_MESSAGE_CHARS}`);
  });

  it('re-enables after a turn errors', () => {
    const script = scriptedTransport();
    renderChat(<Host />, { transport: script.transport });
    fireEvent.change(input(), { target: { value: 'hi' } });
    fireEvent.keyDown(input(), { key: 'Enter' });
    act(() => script.handlers().onError({ code: 'llm_error', message: 'down' }));

    fireEvent.change(input(), { target: { value: 'retry' } });
    expect(screen.getByTestId('chat-send')).toBeEnabled();
    expect(chat().streaming).toBe(false);
  });
});
