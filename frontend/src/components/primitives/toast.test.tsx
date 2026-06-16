import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Toaster, toast } from './toaster';

describe('Toast Primitive', () => {
  it('announces the toast when fired and is present in viewport', async () => {
    const user = userEvent.setup();
    render(
      <div>
        <button data-testid="btn" onClick={() => toast({ title: 'Saved', description: 'Settings updated' })}>
          Trigger
        </button>
        <Toaster />
      </div>
    );

    // Viewport region is initially empty
    expect(screen.queryByText('Saved')).not.toBeInTheDocument();

    // Click trigger to fire toast
    const btn = screen.getByTestId('btn');
    await user.click(btn);

    // Toast is fired, check content is present
    expect(screen.getByText('Saved')).toBeInTheDocument();
    expect(screen.getByText('Settings updated')).toBeInTheDocument();

    // Viewport region is present
    const viewport = document.querySelector('ol[tabindex="-1"]');
    expect(viewport).toBeInTheDocument();
  });
});
