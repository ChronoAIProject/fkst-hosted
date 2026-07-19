import { describe, it, expect, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter } from 'react-router-dom';
import { TourProvider, useTour, seenKey, markTourSeen } from './tour-context';
import { TourOverlay } from './tour-overlay';

// A probe exposing the tour controller as plain buttons, so the state machine
// can be driven independently of the overlay's DOM. Control buttons are named
// with a `ctl-` prefix so they never collide with the overlay's own
// Skip/Back/Next labels in queries.
function Probe({ userKey }: { userKey?: string }) {
  const tour = useTour();
  return (
    <div>
      <button type="button" onClick={() => tour.start()}>
        ctl-start
      </button>
      <button type="button" onClick={() => userKey && tour.startIfUnseen(userKey)}>
        ctl-maybe
      </button>
      <button type="button" onClick={() => tour.next()}>
        ctl-next
      </button>
      <button type="button" onClick={() => tour.back()}>
        ctl-back
      </button>
      <button type="button" onClick={() => tour.skip()}>
        ctl-skip
      </button>
      <button type="button" onClick={() => tour.finish()}>
        ctl-finish
      </button>
      <span data-testid="active">{String(tour.isActive)}</span>
      <span data-testid="index">{tour.index}</span>
      <span data-testid="total">{tour.total}</span>
    </div>
  );
}

function renderTour(userKey?: string) {
  return render(
    <MemoryRouter>
      <TourProvider>
        <Probe userKey={userKey} />
        <TourOverlay />
      </TourProvider>
    </MemoryRouter>
  );
}

const active = () => screen.getByTestId('active').textContent;
const index = () => screen.getByTestId('index').textContent;

beforeEach(() => {
  window.localStorage.clear();
});

describe('startIfUnseen (auto-prompt)', () => {
  it('starts the tour and records the seen key for the login', async () => {
    const user = userEvent.setup();
    renderTour('octocat');
    expect(active()).toBe('false');

    await user.click(screen.getByRole('button', { name: 'ctl-maybe' }));

    expect(active()).toBe('true');
    // The welcome modal is on screen…
    expect(screen.getByText('Welcome to fkst')).toBeInTheDocument();
    // …and the seen flag was set the moment it auto-started.
    expect(window.localStorage.getItem(seenKey('octocat'))).not.toBeNull();
  });

  it('does nothing on a second call for the same login', async () => {
    const user = userEvent.setup();
    // Pre-mark as seen: the auto-prompt must not fire again.
    markTourSeen('octocat');
    renderTour('octocat');

    await user.click(screen.getByRole('button', { name: 'ctl-maybe' }));
    expect(active()).toBe('false');
  });
});

describe('start (the ? re-launch path)', () => {
  it('launches regardless of the seen flag', async () => {
    const user = userEvent.setup();
    markTourSeen('octocat');
    renderTour('octocat');

    await user.click(screen.getByRole('button', { name: 'ctl-start' }));
    expect(active()).toBe('true');
    expect(index()).toBe('0');
  });
});

describe('step navigation', () => {
  it('advances with next and steps back with back', async () => {
    const user = userEvent.setup();
    renderTour();

    await user.click(screen.getByRole('button', { name: 'ctl-start' }));
    expect(index()).toBe('0');

    await user.click(screen.getByRole('button', { name: 'ctl-next' }));
    expect(index()).toBe('1');

    await user.click(screen.getByRole('button', { name: 'ctl-back' }));
    expect(index()).toBe('0');
    // Back at step 0 is a floor — it never goes negative.
    await user.click(screen.getByRole('button', { name: 'ctl-back' }));
    expect(index()).toBe('0');
  });

  it('skip closes the tour', async () => {
    const user = userEvent.setup();
    renderTour();
    await user.click(screen.getByRole('button', { name: 'ctl-start' }));
    expect(active()).toBe('true');

    await user.click(screen.getByRole('button', { name: 'ctl-skip' }));
    expect(active()).toBe('false');
  });

  it('finish closes the tour', async () => {
    const user = userEvent.setup();
    renderTour();
    await user.click(screen.getByRole('button', { name: 'ctl-start' }));

    await user.click(screen.getByRole('button', { name: 'ctl-finish' }));
    expect(active()).toBe('false');
  });
});

describe('overlay rendering', () => {
  it('renders the current step title, body and progress', async () => {
    const user = userEvent.setup();
    renderTour();
    await user.click(screen.getByRole('button', { name: 'ctl-start' }));

    // Welcome (modal) step.
    expect(screen.getByText('Welcome to fkst')).toBeInTheDocument();
    expect(screen.getByText(/60-second tour/)).toBeInTheDocument();
    const total = screen.getByTestId('total').textContent;
    expect(screen.getByText(`1 / ${total}`)).toBeInTheDocument();
  });

  it('degrades to a centered card when the spotlight target is absent', async () => {
    const user = userEvent.setup();
    renderTour();
    await user.click(screen.getByRole('button', { name: 'ctl-start' }));
    // Advance to the canvas spotlight step; its `[data-tour="canvas"]` target
    // is not in this harness's DOM, so the overlay must fall back to a centered
    // card rather than throwing.
    await user.click(screen.getByRole('button', { name: 'ctl-next' }));

    expect(screen.getByTestId('tour-spotlight')).toBeInTheDocument();
    expect(screen.getByText('The canvas')).toBeInTheDocument();
    const total = screen.getByTestId('total').textContent;
    expect(screen.getByText(`2 / ${total}`)).toBeInTheDocument();
  });
});
