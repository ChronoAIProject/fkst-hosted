import { describe, it, expect, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import { Dashboard } from './dashboard';
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

  it('shows the signed-in stub when a token is present', () => {
    window.localStorage.setItem('fkst-gh-access', 'ghu_x');
    renderDashboard();
    expect(screen.getByText('Dashboard coming next')).toBeInTheDocument();
  });
});
