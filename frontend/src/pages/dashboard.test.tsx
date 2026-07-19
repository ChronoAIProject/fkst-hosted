import { describe, it, expect, beforeEach } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { Dashboard, formatSgt } from './dashboard';
import { AuthProvider } from '@/lib/auth/github-auth';

function renderDashboard() {
  return render(
    <AuthProvider>
      <Dashboard />
    </AuthProvider>
  );
}

describe('Dashboard', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.location.hash = '';
  });

  it('prompts sign-in when unauthenticated', () => {
    renderDashboard();
    expect(screen.getByText('Sign in to view your dashboard')).toBeInTheDocument();
  });

  it('treats the same-origin default as configured (no docs-only note)', () => {
    // VITE_FKST_API_BASE unset = SAME-ORIGIN (the deployable topology), so a
    // signed-in user gets the real dashboard surface, never the docs-only note
    // (that state now requires the explicit VITE_FKST_DOCS_ONLY=true build).
    window.localStorage.setItem('fkst-gh-access', 'ghu_x');
    renderDashboard();
    expect(
      screen.queryByText('The dashboard backend is not configured for this deployment yet.')
    ).not.toBeInTheDocument();
  });

  it('renders a slug-specific OAuth error and dismisses it', () => {
    // The callback delivered a real OAuth slug in the fragment; the gate maps it
    // to specific copy and offers a Dismiss control (the sign-in button is the
    // try-again path).
    window.location.hash = '#gh_error=access_denied';
    renderDashboard();
    expect(
      screen.getByText(/cancelled or denied the GitHub authorization/)
    ).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Dismiss' }));
    expect(
      screen.queryByText(/cancelled or denied the GitHub authorization/)
    ).not.toBeInTheDocument();
  });

  it('falls back to the generic message for an unrecognized OAuth slug', () => {
    window.location.hash = '#gh_error=some_unknown_slug';
    renderDashboard();
    expect(
      screen.getByText('Sign-in was cancelled or failed. Please try again.')
    ).toBeInTheDocument();
    // The raw slug stays visible so an unmapped failure is still diagnosable.
    expect(screen.getByText('some_unknown_slug')).toBeInTheDocument();
  });
});

describe('formatSgt', () => {
  it('renders an epoch as Singapore time with an SGT suffix', () => {
    // Epoch 0 is 1970-01-01 08:00 in SGT (UTC+8).
    const s = formatSgt(0, 'en');
    expect(s).toContain('SGT');
    expect(s).toContain('1970');
  });
});
