import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { Introduction } from './introduction';
import { MANUAL_URL } from '@/i18n/literals';

function renderIntro() {
  return render(
    <MemoryRouter>
      <Introduction />
    </MemoryRouter>
  );
}

describe('Introduction', () => {
  it('renders the two-line v2 hero headline', () => {
    renderIntro();
    const h1 = screen.getByRole('heading', { level: 1 });
    expect(h1).toHaveTextContent('Open an issue.');
    expect(h1).toHaveTextContent('Get a pull request.');
  });

  it('states the one-line lede', () => {
    renderIntro();
    expect(
      screen.getByText(/No infrastructure, nothing to learn/i)
    ).toBeInTheDocument();
  });

  it('links the primary CTA to Get Started and the secondary to the manual', () => {
    renderIntro();
    expect(screen.getByRole('link', { name: 'Get started' })).toHaveAttribute(
      'href',
      '/get-started'
    );
    expect(screen.getByRole('link', { name: /operator manual/i })).toHaveAttribute(
      'href',
      MANUAL_URL
    );
  });

  it('exposes the flow-line labels to assistive tech, hiding only the ornaments', () => {
    const { container } = renderIntro();
    // The labels carry real information ("trigger issue → live session → a PR
    // per task") and must stay in the accessibility tree…
    for (const label of ['trigger issue', 'live session', 'a PR per task']) {
      expect(screen.getByText(label).closest('[aria-hidden="true"]')).toBeNull();
    }
    // …while the connector hairlines/dots are purely decorative.
    expect(container.querySelectorAll('[aria-hidden="true"]').length).toBeGreaterThanOrEqual(2);
  });
});
