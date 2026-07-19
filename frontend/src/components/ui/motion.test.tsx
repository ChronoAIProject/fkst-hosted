import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import {
  RouteTransition,
  FadeSwap,
  Reveal,
  OverlayPresence,
  StaggerItem,
  staggerStyle,
  MOTION_EASE,
  STAGGER_STEP_MS,
} from './motion';

// Hoisted mock state so the (hoisted) vi.mock factory can read it: `reduced`
// toggles the mocked useReducedMotion; `calls` records every motion element's
// props so we can assert the exact enter keyframe each primitive requests.
const mockState = vi.hoisted(() => ({
  reduced: false,
  calls: [] as Array<Record<string, unknown>>,
}));

// framer-motion in jsdom does not synchronously flush animated values into the
// inline `style` attribute, so reading `style.opacity` after render is
// unreliable. Instead we stub `motion.*` with plain elements that RECORD the
// props (initial/animate/exit) — a deterministic view of what the primitive
// asked framer to do — while still rendering children and honoring onClick /
// role so behavioral assertions keep working. AnimatePresence and
// useReducedMotion (mocked) stay wired through.
vi.mock('framer-motion', async (importOriginal) => {
  const actual = await importOriginal<typeof import('framer-motion')>();
  const React = await import('react');
  const cache = new Map<string, React.ComponentType<Record<string, unknown>>>();
  const stubFor = (tag: string) => {
    const existing = cache.get(tag);
    if (existing) return existing;
    // forwardRef because framer's popLayout wrapper passes a ref to its child.
    const Stub = React.forwardRef<unknown, Record<string, unknown>>((props, ref) => {
      mockState.calls.push(props);
      // Strip framer-only props so React does not warn on unknown DOM attrs.
      const { initial, animate, exit, transition, children, ...rest } = props;
      void initial;
      void animate;
      void exit;
      void transition;
      return React.createElement(tag, { ...rest, ref }, children as React.ReactNode);
    }) as unknown as React.ComponentType<Record<string, unknown>>;
    cache.set(tag, Stub);
    return Stub;
  };
  const motion = new Proxy(
    {},
    { get: (_t, key: string) => stubFor(key) }
  ) as typeof actual.motion;
  return { ...actual, motion, useReducedMotion: () => mockState.reduced };
});

beforeEach(() => {
  mockState.reduced = false;
  mockState.calls = [];
});

/** The recorded props of the motion element wrapping the given text child. */
function callWithText(text: string): Record<string, unknown> {
  const call = mockState.calls.find((c) => c.children === text);
  if (!call) throw new Error(`no motion element rendered around "${text}"`);
  return call;
}

describe('RouteTransition', () => {
  it('requests an opacity+lift enter under normal motion and renders children', () => {
    render(<RouteTransition k="/a">route-a</RouteTransition>);
    expect(screen.getByText('route-a')).toBeInTheDocument();
    expect(callWithText('route-a').initial).toEqual({ opacity: 0, y: 6 });
  });

  it('mounts at the final state (initial=false) under reduced motion', () => {
    mockState.reduced = true;
    render(<RouteTransition k="/a">route-a</RouteTransition>);
    expect(screen.getByText('route-a')).toBeInTheDocument();
    // initial={false} tells framer to skip the enter animation entirely.
    expect(callWithText('route-a').initial).toBe(false);
    expect(callWithText('route-a').animate).toEqual({ opacity: 1, y: 0 });
  });
});

describe('FadeSwap', () => {
  it('crossfades under normal motion and swaps instantly under reduced motion', () => {
    const { rerender } = render(<FadeSwap k="one">body-one</FadeSwap>);
    expect(screen.getByText('body-one')).toBeInTheDocument();
    expect(callWithText('body-one').initial).toEqual({ opacity: 0 });

    mockState.reduced = true;
    mockState.calls = [];
    rerender(<FadeSwap k="two">body-two</FadeSwap>);
    expect(screen.getByText('body-two')).toBeInTheDocument();
    expect(callWithText('body-two').initial).toBe(false);
  });
});

describe('Reveal', () => {
  it('renders the body when open and nothing when closed', () => {
    const { rerender } = render(<Reveal open={false}>disclosure</Reveal>);
    expect(screen.queryByText('disclosure')).not.toBeInTheDocument();

    rerender(<Reveal open>disclosure</Reveal>);
    expect(screen.getByText('disclosure')).toBeInTheDocument();
    expect(callWithText('disclosure').initial).toEqual({ height: 0, opacity: 0 });
  });

  it('opens at full height instantly under reduced motion', () => {
    mockState.reduced = true;
    render(<Reveal open>disclosure</Reveal>);
    expect(screen.getByText('disclosure')).toBeInTheDocument();
    expect(callWithText('disclosure').initial).toBe(false);
    expect(callWithText('disclosure').animate).toEqual({ height: 'auto', opacity: 1 });
  });
});

describe('OverlayPresence', () => {
  it('renders nothing while closed', () => {
    render(
      <OverlayPresence open={false} variant="modal">
        overlay-body
      </OverlayPresence>
    );
    expect(screen.queryByText('overlay-body')).not.toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('renders a labelled panel with a scale-in enter (modal, normal motion)', () => {
    render(
      <OverlayPresence open variant="modal" label="Test dialog">
        overlay-body
      </OverlayPresence>
    );
    expect(screen.getByRole('dialog', { name: 'Test dialog' })).toBeInTheDocument();
    expect(screen.getByText('overlay-body')).toBeInTheDocument();
    // Panel enters scaled-down + lifted; scrim fades from transparent.
    expect(callWithText('overlay-body').initial).toEqual({ opacity: 0, scale: 0.96, y: 10 });
    const scrim = mockState.calls.find((c) => String(c.className).includes('fixed inset-0'));
    expect(scrim?.initial).toEqual({ opacity: 0 });
  });

  it('uses a right-edge slide for the drawer variant', () => {
    render(
      <OverlayPresence open variant="drawer" label="Test drawer">
        overlay-body
      </OverlayPresence>
    );
    expect(callWithText('overlay-body').initial).toEqual({ x: '100%' });
  });

  it('shows the final state instantly under reduced motion', () => {
    mockState.reduced = true;
    render(
      <OverlayPresence open variant="drawer" label="Test drawer">
        overlay-body
      </OverlayPresence>
    );
    expect(screen.getByRole('dialog', { name: 'Test drawer' })).toBeInTheDocument();
    // Both scrim and panel mount at rest — no fade, no slide.
    for (const call of mockState.calls) expect(call.initial).toBe(false);
  });

  it('fires onBackdropClick from the scrim but not from the panel', () => {
    const onBackdropClick = vi.fn();
    render(
      <OverlayPresence open variant="modal" label="Dlg" onBackdropClick={onBackdropClick}>
        overlay-body
      </OverlayPresence>
    );
    // Panel clicks are swallowed (stopPropagation) so they never close.
    fireEvent.click(screen.getByText('overlay-body'));
    expect(onBackdropClick).not.toHaveBeenCalled();

    // Clicking the scrim itself closes.
    const scrim = screen.getByRole('dialog').parentElement as HTMLElement;
    fireEvent.click(scrim);
    expect(onBackdropClick).toHaveBeenCalledTimes(1);
  });
});

describe('StaggerItem / staggerStyle', () => {
  it('applies the anim-row-in class and an index-based --stagger delay', () => {
    render(
      <StaggerItem index={3} className="extra">
        row
      </StaggerItem>
    );
    const el = screen.getByText('row');
    expect(el).toHaveClass('anim-row-in');
    expect(el).toHaveClass('extra');
    expect(el.style.getPropertyValue('--stagger')).toBe(`${3 * STAGGER_STEP_MS}ms`);
  });

  it('computes delays and honors a custom step', () => {
    const v = (n: number, step?: number) =>
      (staggerStyle(n, step) as Record<string, string>)['--stagger'];
    expect(v(0)).toBe('0ms');
    expect(v(2)).toBe(`${2 * STAGGER_STEP_MS}ms`);
    expect(v(4, 25)).toBe('100ms');
  });

  it('degrades invalid indices to zero delay rather than emitting NaN', () => {
    const v = (n: number) => (staggerStyle(n) as Record<string, string>)['--stagger'];
    expect(v(Number.NaN)).toBe('0ms');
    expect(v(-5)).toBe('0ms');
    expect(v(Infinity)).toBe('0ms');
  });
});

describe('exported constants', () => {
  it('exposes the shared curve matching index.css cubic-bezier(0.2,0.7,0.3,1)', () => {
    expect(MOTION_EASE).toEqual([0.2, 0.7, 0.3, 1]);
  });
});
