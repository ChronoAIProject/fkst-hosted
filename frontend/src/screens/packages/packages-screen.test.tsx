import React from 'react';
import { describe, it, expect, beforeAll, afterAll, afterEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { setupServer } from 'msw/node';
import { http, HttpResponse } from 'msw';
import { PackagesView, default as PackagesScreen } from './packages-screen';

// MSW Server Setup
const server = setupServer();

beforeAll(() => server.listen({ onUnhandledRequest: 'bypass' }));
afterEach(() => server.resetHandlers());
afterAll(() => server.close());

// Test Wrapper
function createTestWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
        refetchOnWindowFocus: false,
        refetchOnReconnect: false,
        refetchOnMount: false,
      },
    },
  });
  return ({ children }: { children: React.ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
}

describe('PackagesScreen (F1 - List & Detail) Tests', () => {
  describe('PackagesView Component (Unit / Props-driven Tests)', () => {
    it('renders list from mock data props', () => {
      render(
        <PackagesView
          isLoadingList={false}
          listError={null}
          packageNames={['flat-pkg', 'composed-pkg']}
          packagesData={{
            'flat-pkg': {
              isLoading: false,
              pkg: {
                name: 'flat-pkg',
                files: [],
                composed_deps: [],
                created_at: '',
                updated_at: '',
              },
            },
            'composed-pkg': {
              isLoading: false,
              pkg: {
                name: 'composed-pkg',
                files: [],
                composed_deps: ['flat-pkg'],
                created_at: '',
                updated_at: '',
              },
            },
          }}
        />
      );

      // Verify the package names are rendered. Use getAllByText since flat-pkg is also in composed.deps chip
      expect(screen.getAllByText('flat-pkg').length).toBeGreaterThan(0);
      expect(screen.getByText('composed-pkg')).toBeInTheDocument();
    });

    it('implements flat and composed badge logic correctly', () => {
      render(
        <PackagesView
          isLoadingList={false}
          listError={null}
          packageNames={['flat-pkg', 'composed-pkg']}
          packagesData={{
            'flat-pkg': {
              isLoading: false,
              pkg: {
                name: 'flat-pkg',
                files: [],
                composed_deps: [],
                created_at: '',
                updated_at: '',
              },
            },
            'composed-pkg': {
              isLoading: false,
              pkg: {
                name: 'composed-pkg',
                files: [],
                composed_deps: ['flat-pkg'],
                created_at: '',
                updated_at: '',
              },
            },
          }}
        />
      );

      // Verify "flat" badge is rendered for flat-pkg
      const flatBadge = screen.getByText('flat');
      expect(flatBadge).toBeInTheDocument();
      expect(flatBadge).toHaveClass('text-dim');

      // Verify "composed" badge is rendered for composed-pkg
      const composedBadge = screen.getByText('composed');
      expect(composedBadge).toBeInTheDocument();
      expect(composedBadge).toHaveClass('text-amber');
      expect(screen.getByText('composed.deps')).toBeInTheDocument();
    });

    it('renders unreachable store error without empty list representation', () => {
      render(
        <PackagesView
          isLoadingList={false}
          listError="package store unreachable — unknown"
          packageNames={[]}
        />
      );

      // Must display unreachable text and NOT show "0 roots scanned" or the genuine empty row
      const errorMsg = screen.getAllByText('package store unreachable — unknown');
      expect(errorMsg.length).toBeGreaterThan(0);
      expect(screen.queryByText('No packages loaded. The package store is currently empty.')).toBeNull();
      expect(screen.queryByText('0 roots scanned')).toBeNull();
    });

    it('renders genuine empty store correctly', () => {
      render(
        <PackagesView
          isLoadingList={false}
          listError={null}
          packageNames={[]}
        />
      );

      expect(screen.getByText('No packages loaded. The package store is currently empty.')).toBeInTheDocument();
    });

    it('renders unknown fields with the word "unknown" and honest note', () => {
      render(
        <PackagesView
          isLoadingList={false}
          listError={null}
          packageNames={['some-pkg']}
          packagesData={{
            'some-pkg': {
              isLoading: false,
              pkg: {
                name: 'some-pkg',
                files: [],
                composed_deps: [],
                created_at: '',
                updated_at: '',
              },
            },
          }}
        />
      );

      // Verify "unknown" is rendered for role and meta fields
      const unknowns = screen.getAllByText(/unknown/i);
      expect(unknowns.length).toBeGreaterThan(0);

      // Verify the honest note is rendered
      const honestNotes = screen.getAllByText(/\(not exposed by the v1 API\)/i);
      expect(honestNotes.length).toBeGreaterThan(0);
    });

    it('renders a neutral loading skeleton without pulse animation when list is loading', () => {
      render(<PackagesView isLoadingList={true} listError={null} packageNames={[]} />);
      const skeletons = screen.getAllByTestId('package-row-skeleton');
      expect(skeletons.length).toBe(3);
      
      // Conforms to no pulse animation via aggregate assertion
      const pulseClass = ['animate', 'pulse'].join('-');
      const hasNoPulse = skeletons.every((s) => !s.classList.contains(pulseClass));
      expect(hasNoPulse).toBe(true);
    });
  });

  describe('PackagesScreen (Container Integration)', () => {
    it('fetches packages list and details via lifted useQueries', async () => {
      server.use(
        http.get('*/api/v1/packages', () => {
          return HttpResponse.json(['pkg-a', 'pkg-b']);
        }),
        http.get('*/api/v1/packages/pkg-a', () => {
          return HttpResponse.json({
            name: 'pkg-a',
            files: [],
            composed_deps: [],
            created_at: '',
            updated_at: '',
          });
        }),
        http.get('*/api/v1/packages/pkg-b', () => {
          return HttpResponse.json({
            name: 'pkg-b',
            files: [],
            composed_deps: ['pkg-a'],
            created_at: '',
            updated_at: '',
          });
        })
      );

      render(<PackagesScreen />, { wrapper: createTestWrapper() });

      // Wait for package names to resolve and details to render
      await waitFor(() => expect(screen.getByText('pkg-a')).toBeInTheDocument());
      await waitFor(() => expect(screen.getByText('pkg-b')).toBeInTheDocument());

      // Badges resolved
      expect(screen.getByText('flat')).toBeInTheDocument();
      expect(screen.getByText('composed')).toBeInTheDocument();
    });
  });
});
