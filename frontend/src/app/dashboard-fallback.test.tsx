import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { DashboardFallback, OperationsFallback } from './index';
import { NotFound } from '../pages/not-found';
import { ErrorFallbackView } from '../components/ui/error-boundary';

describe('DashboardFallback', () => {
  it('renders a labeled route skeleton with shimmer blocks (never a blank area)', () => {
    render(<DashboardFallback />);

    const status = screen.getByRole('status', { name: 'Loading the dashboard…' });
    expect(status).toBeInTheDocument();
    // The shimmer vocabulary from index.css carries the loading animation.
    expect(status.querySelectorAll('.anim-shimmer').length).toBeGreaterThan(0);
  });
});

describe('OperationsFallback', () => {
  it('renders a labeled, page-shaped skeleton so the fixed-height route never jumps', () => {
    render(<OperationsFallback />);

    const status = screen.getByRole('status', { name: 'Loading operations' });
    expect(status).toBeInTheDocument();
    expect(status.querySelectorAll('.anim-shimmer').length).toBeGreaterThan(0);
    // The route is fixed-height, so the skeleton must fill it rather than
    // collapsing and letting the layout resize when the real page mounts.
    expect(status.className).toContain('h-full');
  });
});

// The router swaps these two elements in for a missing path / a render throw
// respectively; both must be self-contained, friendly, and never blank.
describe('router fallback elements', () => {
  it('the 404 view names the unmatched path and links home', () => {
    render(
      <MemoryRouter initialEntries={['/does/not/exist']}>
        <NotFound />
      </MemoryRouter>
    );

    expect(
      screen.getByRole('heading', { level: 1, name: 'This page does not exist' })
    ).toBeInTheDocument();
    // The exact missing path is surfaced (as a mono chip via <Rich>), not hidden
    // behind a silent redirect.
    expect(screen.getByText('/does/not/exist')).toBeInTheDocument();
    expect(screen.getByRole('link', { name: /home/i })).toHaveAttribute('href', '/');
  });

  it('the error fallback shows a friendly alert with a reload action', () => {
    render(<ErrorFallbackView detail="boom detail" />);

    const alert = screen.getByRole('alert');
    expect(alert).toBeInTheDocument();
    expect(screen.getByText('Something went wrong')).toBeInTheDocument();
    expect(screen.getByText('boom detail')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Reload the page' })).toBeInTheDocument();
  });
});
