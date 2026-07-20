import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import { AuthProvider } from '@/lib/auth/github-auth';
import { BroaderOAuthProvider } from '@/lib/auth/broader-oauth';

// The docs-only degrade state is opt-in (VITE_FKST_DOCS_ONLY=true at build
// time). `import.meta.env` is baked at module load, so the flag is exercised
// by mocking the env module for this file only — the sibling dashboard tests
// keep the real (configured) default.
vi.mock('@/lib/env', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/lib/env')>()),
  API_CONFIGURED: false,
}));

// Import AFTER the mock so the component sees the docs-only flag.
const { Dashboard } = await import('./dashboard');

describe('Dashboard (docs-only build)', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.location.hash = '';
  });

  it('shows the not-configured note instead of firing doomed API calls', () => {
    window.localStorage.setItem('fkst-gh-access', 'ghu_x');
    render(
      <AuthProvider>
        <BroaderOAuthProvider>
          <Dashboard />
        </BroaderOAuthProvider>
      </AuthProvider>
    );
    expect(
      screen.getByText('The dashboard backend is not configured for this deployment yet.')
    ).toBeInTheDocument();
  });
});
