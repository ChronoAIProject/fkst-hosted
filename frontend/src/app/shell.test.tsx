import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { MemoryRouter, Routes, Route } from 'react-router-dom';
import { Shell, nextCondensed } from './shell';

function renderShell() {
  return render(
    <MemoryRouter initialEntries={['/']}>
      <Routes>
        <Route element={<Shell />}>
          <Route index element={<div>home content</div>} />
        </Route>
      </Routes>
    </MemoryRouter>
  );
}

describe('Shell', () => {
  it('exposes the two primary nav tabs and renders the outlet', () => {
    renderShell();
    expect(screen.getByRole('link', { name: 'Introduction' })).toBeInTheDocument();
    // "Get Started" appears as the nav tab, the header CTA, and the footer link.
    expect(screen.getAllByRole('link', { name: /get started/i }).length).toBeGreaterThanOrEqual(2);
    expect(screen.getByText('home content')).toBeInTheDocument();
  });

  it('has no backend/app tabs left over', () => {
    renderShell();
    expect(screen.queryByRole('link', { name: 'Goals' })).not.toBeInTheDocument();
    expect(screen.queryByRole('link', { name: 'Packages' })).not.toBeInTheDocument();
  });
});

describe('nextCondensed', () => {
  it('condenses past 140px and expands below 40px, holding within the band', () => {
    expect(nextCondensed(false, 200)).toBe(true);
    expect(nextCondensed(true, 10)).toBe(false);
    expect(nextCondensed(true, 100)).toBe(true); // hysteresis holds previous
    expect(nextCondensed(false, 100)).toBe(false);
  });
});
