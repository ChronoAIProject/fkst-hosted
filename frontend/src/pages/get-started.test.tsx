import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { GetStarted } from './get-started';

function renderGetStarted() {
  return render(
    <MemoryRouter>
      <GetStarted />
    </MemoryRouter>
  );
}

describe('GetStarted', () => {
  it('renders the page title and the install step', () => {
    renderGetStarted();
    expect(
      screen.getByRole('heading', { level: 1, name: /Drive fkst-hosted with GitHub issues/i })
    ).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: /Install the GitHub App/i })).toBeInTheDocument();
  });

  it('documents the required trigger parameters', () => {
    renderGetStarted();
    // Each heading appears at least in the field reference (some are also
    // referenced in prose), so assert presence rather than uniqueness.
    expect(screen.getAllByText('### Session Name').length).toBeGreaterThan(0);
    expect(screen.getAllByText('### Packages').length).toBeGreaterThan(0);
    expect(screen.getAllByText('### Work Label').length).toBeGreaterThan(0);
  });

  it('explains the package-reference grammar and log access', () => {
    renderGetStarted();
    expect(screen.getAllByText(/owner\/repo@ref:path/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/\/api\/v1\/logs/).length).toBeGreaterThan(0);
  });
});
