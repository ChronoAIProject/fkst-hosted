import React from 'react';
import { describe, it, expect, beforeAll, afterAll, afterEach, vi } from 'vitest';
import { render, screen, waitFor, fireEvent } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { setupServer } from 'msw/node';
import { http, HttpResponse } from 'msw';
import { PackagesView, default as PackagesScreen, deriveTopology } from './packages-screen';
import * as toaster from '../../components/primitives/toaster';
import { SessionRegistryProvider, useSessionRegistry } from '../../lib/hooks/session-registry';

// MSW Server Setup
const server = setupServer();

beforeAll(() => server.listen({ onUnhandledRequest: 'warn' }));
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
    <QueryClientProvider client={queryClient}>
      <SessionRegistryProvider>{children}</SessionRegistryProvider>
    </QueryClientProvider>
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

    describe('deriveTopology unit tests', () => {
      it('correctly extracts departments and raisers from matching file paths', () => {
        const pkg = {
          name: 'test-pkg',
          files: [
            { path: 'departments/dept-a/main.lua', content: '' },
            { path: 'departments/dept-b/main.lua', content: '' },
            { path: 'raisers/raiser-x.lua', content: '' },
            { path: 'raisers/raiser-y.lua', content: '' },
          ],
          composed_deps: [],
          created_at: '',
          updated_at: '',
        };

        const result = deriveTopology(pkg);
        expect(result.departments).toEqual(['dept-a', 'dept-b']);
        expect(result.raisers).toEqual(['raiser-x', 'raiser-y']);
      });

      it('ignores non-matching or nested file paths', () => {
        const pkg = {
          name: 'test-pkg',
          files: [
            { path: 'departments/dept-a/sub/main.lua', content: '' },
            { path: 'departments/main.lua', content: '' },
            { path: 'raisers/sub/raiser-x.lua', content: '' },
            { path: 'other/main.lua', content: '' },
          ],
          composed_deps: [],
          created_at: '',
          updated_at: '',
        };

        const result = deriveTopology(pkg);
        expect(result.departments).toEqual([]);
        expect(result.raisers).toEqual([]);
      });
    });

    describe('Topology Composed Graph & Read/Write Tri-Panel render tests', () => {
      it('renders unknown wiring with note and tri-panel contents', () => {
        render(
          <PackagesView
            isLoadingList={false}
            listError={null}
            packageNames={['my-pkg']}
            selectedPkgName="my-pkg"
            packagesData={{
              'my-pkg': {
                isLoading: false,
                pkg: {
                  name: 'my-pkg',
                  files: [
                    { path: 'departments/dept-z/main.lua', content: '' },
                    { path: 'raisers/raiser-w.lua', content: '' },
                  ],
                  composed_deps: ['other-pkg'],
                  created_at: '',
                  updated_at: '',
                },
              },
            }}
          />
        );

        // Verify select dropdown header info
        expect(screen.getByText(/nodes = departments · edges = queues/)).toBeInTheDocument();

        // Verify derived raiser name and fallback note
        expect(screen.getByText('raiser-w')).toBeInTheDocument();
        expect(screen.getAllByText(/\(declared in Lua, not parsed\)/i).length).toBeGreaterThan(0);

        // Verify derived department name and wiring unknown note
        expect(screen.getAllByText('dept-z').length).toBeGreaterThan(0);
        expect(screen.getAllByText(/\(wiring declared in Lua; not parsed by this console\)/i).length).toBeGreaterThan(0);

        // Verify tri-panel content
        expect(screen.getByText('Read-only')).toBeInTheDocument();
        expect(screen.getByText('FE manages')).toBeInTheDocument();
        expect(screen.getByText('Business writes')).toBeInTheDocument();
        expect(screen.getByText('runtime read-only')).toBeInTheDocument();
        expect(screen.getByText('applied via restart')).toBeInTheDocument();
        expect(screen.getByText('REAL posture required')).toBeInTheDocument();
      });
    });
  });

// Registry Initializer for Testing
function RegistryInitializer({
  packageName,
  sessionId,
  children,
}: {
  packageName: string;
  sessionId: string;
  children: React.ReactNode;
}) {
  const { registerSession } = useSessionRegistry();
  React.useEffect(() => {
    registerSession(packageName, sessionId);
  }, [packageName, sessionId, registerSession]);
  return <>{children}</>;
}

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

    it('renders Switch toggle, defaults to enabled for non-special package, and shows target-state note initially', async () => {
      server.use(
        http.get('*/api/v1/packages', () => {
          return HttpResponse.json(['pkg-a']);
        }),
        http.get('*/api/v1/packages/pkg-a', () => {
          return HttpResponse.json({
            name: 'pkg-a',
            files: [],
            composed_deps: [],
            created_at: '',
            updated_at: '',
          });
        })
      );

      render(<PackagesScreen />, { wrapper: createTestWrapper() });

      // Wait for details query to resolve by finding the switch toggle
      const switchToggle = await screen.findByLabelText('Toggle target state for pkg-a');
      expect(switchToggle).toBeInTheDocument();

      // "enabled" label and target-state intent warning note should be present initially
      expect(screen.getByText('enabled')).toBeInTheDocument();
      expect(
        screen.getByText('target state — applies via session restart; no enable endpoint in v1')
      ).toBeInTheDocument();

      // Toggle it
      fireEvent.click(switchToggle);

      // Verify "disabled" label is rendered, and target-state note remains
      expect(screen.getByText('disabled')).toBeInTheDocument();
      expect(
        screen.getByText('target state — applies via session restart; no enable endpoint in v1')
      ).toBeInTheDocument();
    });

    describe('Session Cycling Flow (F4)', () => {
      it('disables Apply button and shows gap copy when session ID is not known', async () => {
        server.use(
          http.get('*/api/v1/packages', () => {
            return HttpResponse.json(['pkg-b']);
          }),
          http.get('*/api/v1/packages/pkg-b', () => {
            return HttpResponse.json({
              name: 'pkg-b',
              files: [
                { path: 'departments/dept-b/main.lua', content: '' }
              ],
              composed_deps: [],
              created_at: '',
              updated_at: '',
            });
          })
        );

        render(<PackagesScreen />, { wrapper: createTestWrapper() });

        // Wait for details to load by waiting for the gap copy to appear
        const gapCopy = await screen.findByText(/pkg-b · current session id not exposed/i);
        expect(gapCopy).toBeInTheDocument();

        // Apply button should be disabled
        const applyBtn = screen.getByRole('button', { name: /Apply changes to pkg-b · stop & restart session/i });
        expect(applyBtn).toBeDisabled();
      });

      it('executes happy path stop & restart flow (3-phase progress)', async () => {
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
        const customWrapper = ({ children }: { children: React.ReactNode }) => (
          <QueryClientProvider client={queryClient}>
            <SessionRegistryProvider>{children}</SessionRegistryProvider>
          </QueryClientProvider>
        );

        let stopCalled = false;
        let getSessionCallCount = 0;
        let createCalled = false;

        server.use(
          http.get('*/api/v1/packages', () => {
            return HttpResponse.json(['pkg-b']);
          }),
          http.get('*/api/v1/packages/pkg-b', () => {
            return HttpResponse.json({
              name: 'pkg-b',
              files: [
                { path: 'departments/dept-b/main.lua', content: '' }
              ],
              composed_deps: [],
              created_at: '',
              updated_at: '',
            });
          }),
          http.post('*/api/v1/sessions/session-123/stop', () => {
            stopCalled = true;
            return HttpResponse.json({ message: 'Stop requested' }, { status: 202 });
          }),
          http.get('*/api/v1/sessions/session-123', () => {
            getSessionCallCount++;
            if (getSessionCallCount === 1) {
              return HttpResponse.json({ id: 'session-123', status: 'stopping' });
            } else {
              return HttpResponse.json({ id: 'session-123', status: 'stopped' });
            }
          }),
          http.post('*/api/v1/sessions', async ({ request }) => {
            const body = await request.json() as { package_name: string };
            expect(body.package_name).toBe('pkg-b');
            createCalled = true;
            await new Promise((resolve) => setTimeout(resolve, 50));
            return HttpResponse.json({ id: 'session-456', status: 'pending' }, { status: 201 });
          })
        );

        render(
          <RegistryInitializer packageName="pkg-b" sessionId="session-123">
            <PackagesScreen />
          </RegistryInitializer>,
          { wrapper: customWrapper }
        );

        // Wait for Apply button to be enabled (details resolved & session ID registered)
        const applyBtn = await screen.findByRole('button', { name: /Apply changes to pkg-b · stop & restart session/i });
        await waitFor(() => expect(applyBtn).not.toBeDisabled());

        // Click Apply
        fireEvent.click(applyBtn);

        // Phase 1: stop requested
        await screen.findByText('pkg-b · stop requested (202 ack) →');
        expect(stopCalled).toBe(true);

        // Phase 2: waiting for stopped
        await screen.findByText('pkg-b · waiting for stopped →');

        // Manually refetch the session query to simulate the 2-second poll firing
        await queryClient.refetchQueries({ queryKey: ['sessions', 'session-123'] });

        // Phase 3: starting new session
        await screen.findByText('pkg-b · starting new session →');
        await waitFor(() => expect(createCalled).toBe(true));

        // Returns to idle (no progress indicators)
        await waitFor(() => {
          expect(screen.queryByText(/stop requested/)).toBeNull();
          expect(screen.queryByText(/waiting for stopped/)).toBeNull();
          expect(screen.queryByText(/starting new session/)).toBeNull();
        });
      });

      it('displays conflict copy when create session returns 409', async () => {
        let createCallCount = 0;

        server.use(
          http.get('*/api/v1/packages', () => {
            return HttpResponse.json(['pkg-b']);
          }),
          http.get('*/api/v1/packages/pkg-b', () => {
            return HttpResponse.json({
              name: 'pkg-b',
              files: [
                { path: 'departments/dept-b/main.lua', content: '' }
              ],
              composed_deps: [],
              created_at: '',
              updated_at: '',
            });
          }),
          http.post('*/api/v1/sessions/session-123/stop', () => {
            return HttpResponse.json({ message: 'Stop requested' }, { status: 202 });
          }),
          http.get('*/api/v1/sessions/session-123', () => {
            return HttpResponse.json({ id: 'session-123', status: 'stopped' });
          }),
          http.post('*/api/v1/sessions', () => {
            createCallCount++;
            return HttpResponse.json(
              { error: 'conflict', message: "package already has a live session; its id isn't exposed by the v1 API, so it can't be stopped from here." },
              { status: 409 }
            );
          })
        );

        render(
          <RegistryInitializer packageName="pkg-b" sessionId="session-123">
            <PackagesScreen />
          </RegistryInitializer>,
          { wrapper: createTestWrapper() }
        );

        // Wait for Apply button to be enabled (details resolved & session ID registered)
        const applyBtn = await screen.findByRole('button', { name: /Apply changes to pkg-b · stop & restart session/i });
        await waitFor(() => expect(applyBtn).not.toBeDisabled());

        // Click Apply
        fireEvent.click(applyBtn);

        // Wait for the conflict error text to appear
        expect(
          await screen.findByText("pkg-b · session stopped, but restart failed — package already has a live session; its id isn't exposed by the v1 API, so it can't be stopped from here.")
        ).toBeInTheDocument();
        expect(createCallCount).toBe(1);
      });

      it('allows canceling during the session cycling flow', async () => {
        server.use(
          http.get('*/api/v1/packages', () => {
            return HttpResponse.json(['pkg-b']);
          }),
          http.get('*/api/v1/packages/pkg-b', () => {
            return HttpResponse.json({
              name: 'pkg-b',
              files: [
                { path: 'departments/dept-b/main.lua', content: '' }
              ],
              composed_deps: [],
              created_at: '',
              updated_at: '',
            });
          }),
          http.post('*/api/v1/sessions/session-123/stop', () => {
            return HttpResponse.json({ message: 'Stop requested' }, { status: 202 });
          })
        );

        render(
          <RegistryInitializer packageName="pkg-b" sessionId="session-123">
            <PackagesScreen />
          </RegistryInitializer>,
          { wrapper: createTestWrapper() }
        );

        // Wait for Apply button to be enabled
        const applyBtn = await screen.findByRole('button', { name: /Apply changes to pkg-b · stop & restart session/i });
        await waitFor(() => expect(applyBtn).not.toBeDisabled());

        // Click Apply
        fireEvent.click(applyBtn);

        // Wait for stopping progress indicator to appear
        const stopIndicator = await screen.findByText('pkg-b · stop requested (202 ack) →');
        expect(stopIndicator).toBeInTheDocument();

        // Cancel button should be visible
        const cancelBtn = screen.getByRole('button', { name: /Cancel/i });
        expect(cancelBtn).toBeInTheDocument();

        // Click Cancel
        fireEvent.click(cancelBtn);

        // Verify status copies disappear and Apply button is restored/enabled
        await waitFor(() => {
          expect(screen.queryByText(/stop requested/)).toBeNull();
          expect(screen.queryByText(/waiting for stopped/)).toBeNull();
          expect(screen.queryByText(/starting new session/)).toBeNull();
        });
        expect(applyBtn).not.toBeDisabled();
      });
    });

    describe('Update, Delete and Sharing Mutations', () => {
      it('supports updating package files and dependencies', async () => {
        let updateCalled = false;
        server.use(
          http.get('*/api/v1/packages', () => {
            return HttpResponse.json(['pkg-a']);
          }),
          http.get('*/api/v1/packages/pkg-a', () => {
            return HttpResponse.json({
              name: 'pkg-a',
              files: [{ path: 'departments/dept-a/main.lua', content: 'print(1)' }],
              composed_deps: [],
              created_at: '',
              updated_at: '',
            });
          }),
          http.put('*/api/v1/packages/pkg-a', async ({ request }) => {
            const body = await request.json() as { files: { path: string; content: string }[]; composed_deps?: string[] };
            expect(body.files?.[0]?.content).toBe('print(2)');
            updateCalled = true;
            return HttpResponse.json({
              name: 'pkg-a',
              files: body.files,
              composed_deps: body.composed_deps || [],
              created_at: '',
              updated_at: '',
            }, { status: 200 });
          })
        );

        const toastSpy = vi.spyOn(toaster, 'toast');
        render(<PackagesScreen />, { wrapper: createTestWrapper() });

        // Wait for package detail to resolve
        await screen.findByText('pkg-a');

        // Click Update button on the row
        const updateBtn = screen.getByRole('button', { name: /^Update$/i });
        await userEvent.click(updateBtn);

        // Verify Modal opens with name disabled
        const nameInput = screen.getByLabelText(/Name · unique on create/i);
        expect(nameInput).toBeDisabled();

        // Verify files pre-filled
        const filesTextarea = screen.getByLabelText(/Files · the package root, inline/i) as HTMLTextAreaElement;
        expect(filesTextarea.value).toContain('print(1)');

        // Change files content and submit
        fireEvent.change(filesTextarea, { target: { value: '--- path: departments/dept-a/main.lua\nprint(2)' } });
        const submitBtn = screen.getByRole('button', { name: /Update package/i });
        await userEvent.click(submitBtn);

        await waitFor(() => expect(updateCalled).toBe(true));
        expect(toastSpy).toHaveBeenCalledWith({
          title: 'Updated',
          description: 'Updated — composes on next session start',
        });
      });

      it('supports deleting package after confirmation', async () => {
        let deleteCalled = false;
        server.use(
          http.get('*/api/v1/packages', () => {
            return HttpResponse.json(['pkg-a']);
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
          http.delete('*/api/v1/packages/pkg-a', () => {
            deleteCalled = true;
            return new HttpResponse(null, { status: 200 });
          })
        );

        const toastSpy = vi.spyOn(toaster, 'toast');
        render(<PackagesScreen />, { wrapper: createTestWrapper() });

        // Wait for package to load
        await screen.findByText('pkg-a');

        // Click Delete button on the row
        const deleteBtn = screen.getByRole('button', { name: /^Delete$/i });
        await userEvent.click(deleteBtn);

        // Verify Confirm modal opens
        expect(screen.getByRole('heading', { name: /^Delete package$/i })).toBeInTheDocument();
        expect(screen.getByText(/Are you sure you want to permanently delete the package/i)).toBeInTheDocument();

        // Click Confirm delete
        const confirmDeleteBtn = screen.getByRole('button', { name: /^Delete package$/i });
        await userEvent.click(confirmDeleteBtn);

        await waitFor(() => expect(deleteCalled).toBe(true));
        expect(toastSpy).toHaveBeenCalledWith({
          title: 'Deleted',
          description: 'Successfully deleted package pkg-a',
        });
      });

      it('supports listing, granting and revoking package shares', async () => {
        let createShareCalled = false;
        let deleteShareCalled = false;

        server.use(
          http.get('*/api/v1/packages', () => {
            return HttpResponse.json(['pkg-a']);
          }),
          http.get('*/api/v1/packages/pkg-a', () => {
            return HttpResponse.json({
              name: 'pkg-a',
              files: [{ path: 'departments/dept-a/main.lua', content: '' }],
              composed_deps: [],
              created_at: '',
              updated_at: '',
            });
          }),
          http.get('*/api/v1/packages/pkg-a/shares', () => {
            return HttpResponse.json([
              {
                id: 'share-1',
                package_name: 'pkg-a',
                grantee_kind: 'user',
                grantee_id: 'user-bob',
                level: 'read',
                granted_by: 'owner',
                created_at: '',
              }
            ]);
          }),
          http.post('*/api/v1/packages/pkg-a/shares', async ({ request }) => {
            const body = await request.json() as { grantee_id: string; grantee_kind: string; level: string };
            expect(body.grantee_id).toBe('user-alice');
            expect(body.grantee_kind).toBe('user');
            expect(body.level).toBe('use');
            createShareCalled = true;
            return HttpResponse.json({
              id: 'share-2',
              package_name: 'pkg-a',
              grantee_kind: 'user',
              grantee_id: 'user-alice',
              level: 'use',
              granted_by: 'owner',
              created_at: '',
            }, { status: 201 });
          }),
          http.delete('*/api/v1/packages/pkg-a/shares/share-1', () => {
            deleteShareCalled = true;
            return new HttpResponse(null, { status: 200 });
          })
        );

        const toastSpy = vi.spyOn(toaster, 'toast');
        render(<PackagesScreen />, { wrapper: createTestWrapper() });

        // Wait for package to load
        await screen.findByText('pkg-a');

        // Verify bob's share is listed
        expect(await screen.findByText('user-bob')).toBeInTheDocument();

        // Revoke bob's share
        const revokeBtn = screen.getByRole('button', { name: /Revoke/i });
        await userEvent.click(revokeBtn);
        await waitFor(() => expect(deleteShareCalled).toBe(true));
        expect(toastSpy).toHaveBeenCalledWith({
          title: 'Share revoked',
          description: 'Successfully revoked share for user-bob',
        });

        // Grant share to alice
        const granteeInput = screen.getByLabelText(/Grantee ID/i);
        await userEvent.type(granteeInput, 'user-alice');

        const levelSelect = screen.getByLabelText(/Level/i);
        fireEvent.change(levelSelect, { target: { value: 'use' } });

        const grantBtn = screen.getByRole('button', { name: /Grant/i });
        await userEvent.click(grantBtn);

        await waitFor(() => expect(createShareCalled).toBe(true));
        expect(toastSpy).toHaveBeenCalledWith({
          title: 'Share granted',
          description: 'Successfully shared pkg-a with user-alice',
        });
      });
    });
  });
});

