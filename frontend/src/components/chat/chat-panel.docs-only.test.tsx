import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import { AuthProvider } from '@/lib/auth/github-auth';
import { ToastProvider } from '@/components/ui/toast';
import { ChatProvider } from './chat-context';

// The docs-only degrade state is opt-in (VITE_FKST_DOCS_ONLY=true at build time).
// `import.meta.env` is baked at module load, so the flag is exercised by mocking
// the env module for THIS FILE only — a `doMock` inside a shared file would give
// the dynamically imported components a second copy of the auth context.
vi.mock('@/lib/env', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/lib/env')>()),
  API_CONFIGURED: false,
}));

// Imported AFTER the mock so the components see the docs-only flag.
const { ChatPanel } = await import('./chat-panel');
const { ChatLauncher } = await import('./chat-launcher');

describe('chat surface (docs-only build)', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.sessionStorage.clear();
  });

  it('renders nothing at all without a configured backend', () => {
    // A chat surface with no backend can only disappoint, so it is absent rather
    // than present-and-broken.
    render(
      <ToastProvider>
        <AuthProvider>
          <ChatProvider>
            <ChatPanel />
            <ChatLauncher />
          </ChatProvider>
        </AuthProvider>
      </ToastProvider>
    );
    expect(screen.queryByTestId('chat-panel')).not.toBeInTheDocument();
    expect(screen.queryByTestId('chat-launcher')).not.toBeInTheDocument();
  });
});
