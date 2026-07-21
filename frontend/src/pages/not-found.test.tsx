import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { NotFound } from './not-found';

function renderAt(path: string) {
  return render(
    <MemoryRouter initialEntries={[path]}>
      <NotFound />
    </MemoryRouter>
  );
}

describe('NotFound', () => {
  it('surfaces the exact unmatched path rather than redirecting', () => {
    renderAt('/repos/ghost/page');
    expect(screen.getByText('/repos/ghost/page')).toBeInTheDocument();
    expect(
      screen.getByRole('heading', { level: 1, name: 'This page does not exist' })
    ).toBeInTheDocument();
  });

  it('links back home', () => {
    renderAt('/missing');
    expect(screen.getByRole('link', { name: /home/i })).toHaveAttribute('href', '/');
  });

  it('sets the document title so the tab reflects the 404', () => {
    renderAt('/missing');
    expect(document.title).toBe('FKST — Page not found');
  });
});
