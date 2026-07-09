import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { Introduction } from './introduction';

function renderIntro() {
  return render(
    <MemoryRouter>
      <Introduction />
    </MemoryRouter>
  );
}

describe('Introduction', () => {
  it('renders the hero headline about GitHub-issue-driven sessions', () => {
    renderIntro();
    expect(
      screen.getByRole('heading', { level: 1, name: /driven entirely by GitHub issues/i })
    ).toBeInTheDocument();
  });

  it('lists what the hosted service provides', () => {
    renderIntro();
    expect(screen.getByText('Managed engine on Kubernetes')).toBeInTheDocument();
    expect(screen.getByText('A pull request per task')).toBeInTheDocument();
    expect(screen.getByText('Redacted logs, identity-gated')).toBeInTheDocument();
  });

  it('links to Get Started', () => {
    renderIntro();
    const links = screen.getAllByRole('link', { name: /get started/i });
    expect(links.length).toBeGreaterThan(0);
    expect(links[0]).toHaveAttribute('href', '/get-started');
  });
});
