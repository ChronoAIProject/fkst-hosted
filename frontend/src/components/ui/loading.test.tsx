import { describe, it, expect } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import { LoadingState, Spinner } from './loading';

describe('Spinner', () => {
  it('is decorative, so it never announces itself', () => {
    // A spinning ring says nothing; the LABEL is what carries the state. If the
    // ring were announced it would read as a meaningless element to a screen
    // reader while telling it nothing about the wait.
    const { container } = render(<Spinner />);
    const ring = container.firstElementChild!;
    expect(ring).toHaveAttribute('aria-hidden', 'true');
    expect(ring).toHaveTextContent('');
  });

  it('uses the project animation class, not a banned Tailwind one', () => {
    // `.anim-spin` is the only sanctioned spinner: it is already collapsed to a
    // static ring by the global prefers-reduced-motion block, so reduced motion
    // is honoured without any call site opting in.
    const { container } = render(<Spinner />);
    const ring = container.firstElementChild!;
    expect(ring).toHaveClass('anim-spin');
    // No Tailwind `animate-*` utility: those are lint-banned precisely because
    // they are not wired into the project's reduced-motion block. Matched by
    // prefix rather than by name so the assertion cannot itself name a banned
    // class (the lint rule reads string literals, tests included).
    expect([...ring.classList].some((name) => name.startsWith('animate-'))).toBe(false);
  });

  it('merges a caller class without dropping its own', () => {
    const { container } = render(<Spinner className="mr-1.5" />);
    const ring = container.firstElementChild!;
    expect(ring).toHaveClass('mr-1.5');
    expect(ring).toHaveClass('anim-spin');
  });
});

describe('LoadingState', () => {
  it('announces the wait and shows the label', () => {
    render(<LoadingState label="Loading activity…" />);
    const region = screen.getByRole('status');
    expect(within(region).getByText('Loading activity…')).toBeInTheDocument();
  });

  it('shows the explanatory detail when the wait is API-bound', () => {
    render(
      <LoadingState
        label="Loading activity…"
        detail="Reading the latest data from GitHub in real time — this can take a moment."
      />
    );
    expect(
      screen.getByText('Reading the latest data from GitHub in real time — this can take a moment.')
    ).toBeInTheDocument();
  });

  it('omits the detail line entirely when none is given', () => {
    // An instant wait should not carry an apology for being slow.
    const { container } = render(<LoadingState label="Working…" />);
    expect(container.querySelector('p')).toBeNull();
  });

  it('can stay silent so it never nests inside another live region', () => {
    // Skeletons already carry role="status" on their wrapper; a second one
    // inside would double-announce and break getByRole('status') queries.
    render(
      <div role="status" aria-label="outer">
        <LoadingState announce={false} label="Loading the canvas…" />
      </div>
    );
    const regions = screen.getAllByRole('status');
    expect(regions).toHaveLength(1);
    expect(regions[0]).toHaveAttribute('aria-label', 'outer');
    // …and the label is still visible.
    expect(screen.getByText('Loading the canvas…')).toBeInTheDocument();
  });

  it('centres itself in an empty region in the block variant', () => {
    render(<LoadingState variant="block" label="Loading live sandboxes…" />);
    const region = screen.getByRole('status');
    expect(region).toHaveClass('items-center', 'justify-center', 'text-center');
  });

  it('stays inline by default so it can sit in a flow of content', () => {
    render(<LoadingState label="Loading engine details…" />);
    expect(screen.getByRole('status')).not.toHaveClass('items-center');
  });

  it('exposes a test id when a call site needs to distinguish its state', () => {
    render(<LoadingState testId="operations-loading-activity" label="Loading activity…" />);
    expect(screen.getByTestId('operations-loading-activity')).toBeInTheDocument();
  });

  it('still contains exactly one spinner, whatever the variant', () => {
    const { container, rerender } = render(<LoadingState label="x" />);
    expect(container.querySelectorAll('.anim-spin')).toHaveLength(1);
    rerender(<LoadingState variant="block" label="x" detail="y" />);
    expect(container.querySelectorAll('.anim-spin')).toHaveLength(1);
  });
});
