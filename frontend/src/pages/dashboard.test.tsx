import { describe, it, expect, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
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

  it('shows a not-configured note when signed in but the backend URL is unset', () => {
    // In tests VITE_FKST_API_BASE is unset, so the API is "not configured".
    window.localStorage.setItem('fkst-gh-access', 'ghu_x');
    renderDashboard();
    expect(
      screen.getByText('The dashboard backend is not configured for this deployment yet.')
    ).toBeInTheDocument();
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
