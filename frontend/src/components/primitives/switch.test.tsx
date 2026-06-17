import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Switch } from './switch';

describe('Switch Primitive', () => {
  it('toggles when clicked and handles checked state', async () => {
    const user = userEvent.setup();
    const handleCheckedChange = vi.fn();

    render(<Switch data-testid="switch" onCheckedChange={handleCheckedChange} />);
    const switchEl = screen.getByTestId('switch');

    // Default state: not checked
    expect(switchEl).toHaveAttribute('aria-checked', 'false');

    // Toggle
    await user.click(switchEl);
    expect(switchEl).toHaveAttribute('aria-checked', 'true');
    expect(handleCheckedChange).toHaveBeenLastCalledWith(true);

    // Toggle back
    await user.click(switchEl);
    expect(switchEl).toHaveAttribute('aria-checked', 'false');
    expect(handleCheckedChange).toHaveBeenLastCalledWith(false);
  });

  it('does not toggle when disabled', async () => {
    const user = userEvent.setup();
    const handleCheckedChange = vi.fn();

    render(<Switch data-testid="switch" disabled onCheckedChange={handleCheckedChange} />);
    const switchEl = screen.getByTestId('switch');

    expect(switchEl).toBeDisabled();
    expect(switchEl).toHaveAttribute('aria-checked', 'false');

    // Click disabled switch
    await user.click(switchEl);
    expect(switchEl).toHaveAttribute('aria-checked', 'false');
    expect(handleCheckedChange).not.toHaveBeenCalled();
  });
});
