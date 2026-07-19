import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, fireEvent, act } from '@testing-library/react';
import { ToastProvider, Toaster, useToast } from './toast';

// Hoisted mock state so the (hoisted) vi.mock factory can read it: `reduced`
// toggles the mocked useReducedMotion; `calls` records every motion element's
// props so we can assert the reduced-motion enter contract.
const mockState = vi.hoisted(() => ({
  reduced: false,
  calls: [] as Array<Record<string, unknown>>,
}));

// framer-motion in jsdom does not deterministically resolve exit animations, so
// a real AnimatePresence would leave a dismissed toast lingering and make the
// removal assertions flaky. Stub `motion.*` with plain elements that RECORD
// their props (initial/animate/exit) and make AnimatePresence a passthrough, so
// a toast unmounts the instant it leaves the queue. useReducedMotion is mocked
// from the hoisted flag.
vi.mock('framer-motion', async (importOriginal) => {
  const actual = await importOriginal<typeof import('framer-motion')>();
  const React = await import('react');
  const cache = new Map<string, React.ComponentType<Record<string, unknown>>>();
  const stubFor = (tag: string) => {
    const existing = cache.get(tag);
    if (existing) return existing;
    const Stub = React.forwardRef<unknown, Record<string, unknown>>((props, ref) => {
      mockState.calls.push(props);
      const { initial, animate, exit, transition, layout, children, ...rest } = props;
      void initial;
      void animate;
      void exit;
      void transition;
      void layout;
      return React.createElement(tag, { ...rest, ref }, children as React.ReactNode);
    }) as unknown as React.ComponentType<Record<string, unknown>>;
    cache.set(tag, Stub);
    return Stub;
  };
  const motion = new Proxy({}, { get: (_t, key: string) => stubFor(key) }) as typeof actual.motion;
  const AnimatePresence = ({ children }: { children: React.ReactNode }) =>
    React.createElement(React.Fragment, null, children);
  return { ...actual, motion, AnimatePresence, useReducedMotion: () => mockState.reduced };
});

/** Harness: a button that raises a notice with the given options, plus the
 *  Toaster surface, both inside a provider. */
function Harness({
  options = { kind: 'success' as const, message: 'Saved!' },
  dismissLabel,
}: {
  options?: Parameters<ReturnType<typeof useToast>['show']>[0];
  dismissLabel?: string;
}) {
  return (
    <ToastProvider>
      <Fire options={options} />
      <Toaster dismissLabel={dismissLabel} />
    </ToastProvider>
  );
}

function Fire({ options }: { options: Parameters<ReturnType<typeof useToast>['show']>[0] }) {
  const { show } = useToast();
  return (
    <button type="button" onClick={() => show(options)}>
      raise
    </button>
  );
}

beforeEach(() => {
  mockState.reduced = false;
  mockState.calls = [];
  vi.useFakeTimers();
});

afterEach(() => {
  vi.runOnlyPendingTimers();
  vi.useRealTimers();
});

describe('useToast / show', () => {
  it('renders the message after show() is called', () => {
    render(<Harness options={{ kind: 'success', message: 'Saved!' }} />);
    expect(screen.queryByText('Saved!')).not.toBeInTheDocument();

    fireEvent.click(screen.getByText('raise'));
    expect(screen.getByText('Saved!')).toBeInTheDocument();
  });

  it('rejects an empty / whitespace message and enqueues nothing', () => {
    const { container } = render(<Harness options={{ message: '   ' }} />);
    fireEvent.click(screen.getByText('raise'));

    const region = container.querySelector('[aria-live="polite"]') as HTMLElement;
    // No card rendered; the live region stays empty of notice text.
    expect(region.textContent).toBe('');
  });
});

describe('auto-dismiss', () => {
  it('removes the notice after the default TTL', () => {
    render(<Harness options={{ kind: 'info', message: 'Ping' }} />);
    fireEvent.click(screen.getByText('raise'));
    expect(screen.getByText('Ping')).toBeInTheDocument();

    // Just before the 4s default it is still up...
    act(() => vi.advanceTimersByTime(3999));
    expect(screen.getByText('Ping')).toBeInTheDocument();
    // ...and gone once the window elapses.
    act(() => vi.advanceTimersByTime(1));
    expect(screen.queryByText('Ping')).not.toBeInTheDocument();
  });

  it('honors an explicit ttlMs', () => {
    render(<Harness options={{ message: 'Quick', ttlMs: 1000 }} />);
    fireEvent.click(screen.getByText('raise'));

    act(() => vi.advanceTimersByTime(999));
    expect(screen.getByText('Quick')).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(1));
    expect(screen.queryByText('Quick')).not.toBeInTheDocument();
  });

  it('falls back to the default TTL for an invalid ttlMs', () => {
    render(<Harness options={{ message: 'Bad ttl', ttlMs: -5 }} />);
    fireEvent.click(screen.getByText('raise'));

    // A negative TTL must not dismiss instantly; the default window applies.
    act(() => vi.advanceTimersByTime(3999));
    expect(screen.getByText('Bad ttl')).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(1));
    expect(screen.queryByText('Bad ttl')).not.toBeInTheDocument();
  });
});

describe('manual dismiss', () => {
  it('removes the notice when the × control is clicked', () => {
    render(<Harness options={{ message: 'Close me', ttlMs: 100000 }} dismissLabel="Dismiss" />);
    fireEvent.click(screen.getByText('raise'));
    expect(screen.getByText('Close me')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Dismiss' }));
    expect(screen.queryByText('Close me')).not.toBeInTheDocument();
  });
});

describe('accessibility', () => {
  it('renders a persistent aria-live=polite region, present even when empty', () => {
    const { container } = render(<Harness />);
    const region = container.querySelector('[aria-live="polite"]');
    expect(region).not.toBeNull();
    expect(region?.getAttribute('aria-live')).toBe('polite');
  });
});

describe('reduced motion', () => {
  it('mounts the toast at its final state (initial=false) with no enter animation', () => {
    mockState.reduced = true;
    render(<Harness options={{ message: 'No motion' }} />);
    fireEvent.click(screen.getByText('raise'));

    expect(screen.getByText('No motion')).toBeInTheDocument();
    const card = mockState.calls.find((c) => String(c.className).includes('rounded-card'));
    // initial={false} tells framer to skip the enter animation entirely.
    expect(card?.initial).toBe(false);
    expect(card?.animate).toEqual({ opacity: 1, y: 0, scale: 1 });
  });
});

describe('provider boundary', () => {
  it('throws when useToast is used outside a ToastProvider', () => {
    function Orphan() {
      useToast();
      return null;
    }
    // Silence the expected React error-boundary console noise for this case.
    const spy = vi.spyOn(console, 'error').mockImplementation(() => {});
    expect(() => render(<Orphan />)).toThrow(/ToastProvider/);
    spy.mockRestore();
  });
});
