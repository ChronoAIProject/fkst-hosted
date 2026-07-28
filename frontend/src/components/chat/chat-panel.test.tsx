import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { act, fireEvent, screen } from '@testing-library/react';
import { useChat } from './chat-context';
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
    expect(screen.getByText('FKST // CONCIERGE')).toBeInTheDocument();
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
