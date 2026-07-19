import { describe, it, expect } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { StatusLegend, ViewDescription } from './legend';

// The English catalog is the default (no LanguageProvider needed — see
// i18n/context.tsx), so these are the exact strings the component renders.
const TITLE = 'Legend';
const ROW_NONE = 'Grey — App not installed';
const ROW_INSTALLED = 'Amber — App installed, no active sessions';
const ROW_ACTIVE = 'Blinking amber — active sessions running';

describe('StatusLegend', () => {
  it('renders the title as a collapsed disclosure by default so the list sits higher', () => {
    render(<StatusLegend />);

    const toggle = screen.getByRole('button', { name: TITLE });
    // Closed by default: aria-expanded=false and the swatch rows are absent.
    expect(toggle).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByText(ROW_NONE)).not.toBeInTheDocument();
    expect(screen.queryByText(ROW_INSTALLED)).not.toBeInTheDocument();
    expect(screen.queryByText(ROW_ACTIVE)).not.toBeInTheDocument();
  });

  it('reveals all three status rows when the disclosure is opened', async () => {
    const user = userEvent.setup();
    render(<StatusLegend />);

    const toggle = screen.getByRole('button', { name: TITLE });
    await user.click(toggle);

    expect(toggle).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByText(ROW_NONE)).toBeInTheDocument();
    expect(screen.getByText(ROW_INSTALLED)).toBeInTheDocument();
    expect(screen.getByText(ROW_ACTIVE)).toBeInTheDocument();

    // The toggle controls exactly the revealed list (aria-controls wiring).
    const list = screen.getByRole('list');
    expect(toggle.getAttribute('aria-controls')).toBe(list.getAttribute('id'));
  });

  it('collapses again on a second click (toggle is idempotent per click)', async () => {
    const user = userEvent.setup();
    render(<StatusLegend />);

    const toggle = screen.getByRole('button', { name: TITLE });
    await user.click(toggle);
    expect(screen.getByText(ROW_NONE)).toBeInTheDocument();

    await user.click(toggle);
    expect(toggle).toHaveAttribute('aria-expanded', 'false');
    // Reveal collapses through AnimatePresence, so the body unmounts only once
    // the exit animation resolves — wait for it rather than asserting instantly.
    await waitFor(() => expect(screen.queryByText(ROW_NONE)).not.toBeInTheDocument());
  });
});

describe('ViewDescription', () => {
  it('renders the provided lede text', () => {
    render(<ViewDescription text="You are looking at every account." />);
    expect(screen.getByText('You are looking at every account.')).toBeInTheDocument();
  });

  it('renders empty text without crashing (edge: missing lede)', () => {
    const { container } = render(<ViewDescription text="" />);
    const p = container.querySelector('p');
    expect(p).toBeInTheDocument();
    expect(p?.textContent).toBe('');
  });
});
