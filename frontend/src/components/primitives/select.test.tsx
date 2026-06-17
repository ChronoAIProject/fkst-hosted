import { describe, expect, it, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from './select';

describe('Select Primitive', () => {
  beforeEach(() => {
    window.HTMLElement.prototype.scrollIntoView = vi.fn();
  });

  it('supports keyboard selection', async () => {
    const user = userEvent.setup();
    const handleValueChange = vi.fn();

    render(
      <Select defaultValue="a" onValueChange={handleValueChange}>
        <SelectTrigger data-testid="trigger">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="a">Option A</SelectItem>
          <SelectItem value="b">Option B</SelectItem>
          <SelectItem value="c">Option C</SelectItem>
        </SelectContent>
      </Select>
    );

    const trigger = screen.getByTestId('trigger');

    // Trigger is focused
    trigger.focus();
    expect(trigger).toHaveFocus();

    // Space key opens the dropdown
    await user.keyboard(' ');
    expect(screen.getByRole('listbox')).toBeInTheDocument();

    // Arrow down moves selection and Enter confirms
    await user.keyboard('{ArrowDown}');
    await user.keyboard('{Enter}');

    // Value should change to "b"
    expect(handleValueChange).toHaveBeenCalledWith('b');
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument();
  });
});
