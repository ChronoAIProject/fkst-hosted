import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { DashboardFallback } from './index';

describe('DashboardFallback', () => {
  it('renders a labeled route skeleton with shimmer blocks (never a blank area)', () => {
    render(<DashboardFallback />);

    const status = screen.getByRole('status', { name: 'Loading the dashboard…' });
    expect(status).toBeInTheDocument();
    // The shimmer vocabulary from index.css carries the loading animation.
    expect(status.querySelectorAll('.anim-shimmer').length).toBeGreaterThan(0);
  });
});
