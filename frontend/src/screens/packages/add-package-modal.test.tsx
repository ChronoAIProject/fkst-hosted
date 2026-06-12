import React from 'react';
import { describe, it, expect, beforeAll, afterAll, afterEach, vi } from 'vitest';
import { render, screen, waitFor, fireEvent } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { setupServer } from 'msw/node';
import { http, HttpResponse } from 'msw';
import { AddPackageModal } from './add-package-modal';
import PackagesScreen from './packages-screen';
import * as toaster from '../../components/primitives/toaster';

// MSW setup
const server = setupServer();
beforeAll(() => server.listen({ onUnhandledRequest: 'bypass' }));
afterEach(() => {
  server.resetHandlers();
  vi.restoreAllMocks();
});
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

describe('AddPackageModal (F2) Tests', () => {
  describe('Zod Pre-validation Matrix', () => {
    it('validates package name format and byte length boundaries', async () => {
      render(<AddPackageModal isOpen={true} onOpenChange={() => {}} />, {
        wrapper: createTestWrapper(),
      });

      const nameInput = screen.getByLabelText(/Name · unique, create-only/i);
      const submitBtn = screen.getByRole('button', { name: /Create package/i });

      // Case 1: Invalid characters (spaces)
      await userEvent.clear(nameInput);
      await userEvent.type(nameInput, 'invalid name');
      await userEvent.click(submitBtn);
      expect(
        await screen.findByText(/Package name must contain only alphanumeric characters/i)
      ).toBeInTheDocument();

      // Case 2: Positive boundary: exactly 128 bytes (chars)
      const exactly128 = 'a'.repeat(128);
      await userEvent.clear(nameInput);
      await userEvent.type(nameInput, exactly128);
      await userEvent.click(submitBtn);
      // The name error should not appear
      await waitFor(() => {
        expect(screen.queryByText(/Package name must be 128 bytes or less/i)).toBeNull();
      });

      // Case 3: Negative boundary: 129 bytes (chars)
      const tooLongName = 'a'.repeat(129);
      await userEvent.clear(nameInput);
      await userEvent.type(nameInput, tooLongName);
      await userEvent.click(submitBtn);
      expect(
        await screen.findByText(/Package name must be 128 bytes or less/i)
      ).toBeInTheDocument();
    });

    it('validates engine entry-point rule and files constraints', async () => {
      render(<AddPackageModal isOpen={true} onOpenChange={() => {}} />, {
        wrapper: createTestWrapper(),
      });

      const nameInput = screen.getByLabelText(/Name · unique, create-only/i);
      const filesTextarea = screen.getByLabelText(/Files · the package root, inline/i);
      const submitBtn = screen.getByRole('button', { name: /Create package/i });

      // Fill valid name
      await userEvent.type(nameInput, 'valid-pkg');

      // Case 1: Empty files list
      fireEvent.change(filesTextarea, { target: { value: '' } });
      await userEvent.click(submitBtn);
      expect(await screen.findByText(/At least one file is required/i)).toBeInTheDocument();

      // Case 2: Files without an engine entry point
      fireEvent.change(filesTextarea, { target: { value: '--- path: utils.lua\nlocal x = 1' } });
      await userEvent.click(submitBtn);
      expect(
        await screen.findByText(
          /Package must contain an engine entry point: departments\/<d>\/main\.lua or raisers\/<name>\.lua/i
        )
      ).toBeInTheDocument();

      // Case 3: Valid department entry point
      fireEvent.change(filesTextarea, { target: { value: '--- path: departments/foo/main.lua\nlocal x = 1' } });
      await userEvent.click(submitBtn);
      await waitFor(() => {
        expect(
          screen.queryByText(
            /Package must contain an engine entry point: departments\/<d>\/main\.lua or raisers\/<name>\.lua/i
          )
        ).toBeNull();
      });

      // Case 4: Valid raiser entry point
      fireEvent.change(filesTextarea, { target: { value: '--- path: raisers/bar.lua\nlocal y = 2' } });
      await userEvent.click(submitBtn);
      await waitFor(() => {
        expect(
          screen.queryByText(
            /Package must contain an engine entry point: departments\/<d>\/main\.lua or raisers\/<name>\.lua/i
          )
        ).toBeNull();
      });
    });

    it('enforces 256-pass and 257-fail file count constraints', async () => {
      render(<AddPackageModal isOpen={true} onOpenChange={() => {}} />, {
        wrapper: createTestWrapper(),
      });

      const nameInput = screen.getByLabelText(/Name · unique, create-only/i);
      const filesTextarea = screen.getByLabelText(/Files · the package root, inline/i);
      const submitBtn = screen.getByRole('button', { name: /Create package/i });

      await userEvent.type(nameInput, 'limit-pkg');

      // Construct exactly 256 files including one engine entry point
      let files256 = '--- path: departments/main/main.lua\n-- main code\n';
      for (let i = 1; i < 256; i++) {
        files256 += `--- path: file-${i}.lua\nlocal x = ${i}\n`;
      }

      // Assert 256 files parses and passes file count check
      fireEvent.change(filesTextarea, { target: { value: files256 } });
      await userEvent.click(submitBtn);
      await waitFor(() => {
        expect(screen.queryByText(/Maximum of 256 files allowed/i)).toBeNull();
      });

      // Construct exactly 257 files including one engine entry point
      let files257 = '--- path: departments/main/main.lua\n-- main code\n';
      for (let i = 1; i < 257; i++) {
        files257 += `--- path: file-${i}.lua\nlocal x = ${i}\n`;
      }

      fireEvent.change(filesTextarea, { target: { value: files257 } });
      await userEvent.click(submitBtn);
      expect(await screen.findByText(/Maximum of 256 files allowed/i)).toBeInTheDocument();
    });
  });

  describe('Server Error Mapping & UI Pending State', () => {
    it('surfaces 409 conflict error inline next to the name field', async () => {
      render(<AddPackageModal isOpen={true} onOpenChange={() => {}} />, {
        wrapper: createTestWrapper(),
      });

      const nameInput = screen.getByLabelText(/Name · unique, create-only/i);
      const filesTextarea = screen.getByLabelText(/Files · the package root, inline/i);
      const submitBtn = screen.getByRole('button', { name: /Create package/i });

      server.use(
        http.post('*/api/v1/packages', () => {
          return HttpResponse.json(
            { error: 'conflict', message: 'name already exists (a revision is a new name)' },
            { status: 409 }
          );
        })
      );

      await userEvent.type(nameInput, 'conflict-pkg');
      fireEvent.change(filesTextarea, { target: { value: '--- path: departments/my-dept/main.lua\nreturn {}' } });
      await userEvent.click(submitBtn);

      expect(
        await screen.findByText('name already exists (a revision is a new name)')
      ).toBeInTheDocument();
    });

    it('disables the submit button while the mutation is pending', async () => {
      let resolveMutation!: (value: unknown) => void;
      const mutationPromise = new Promise((resolve) => {
        resolveMutation = resolve;
      });

      server.use(
        http.post('*/api/v1/packages', async () => {
          await mutationPromise;
          return HttpResponse.json({ name: 'pending-pkg' }, { status: 201 });
        })
      );

      render(<AddPackageModal isOpen={true} onOpenChange={() => {}} />, {
        wrapper: createTestWrapper(),
      });

      const nameInput = screen.getByLabelText(/Name · unique, create-only/i);
      const filesTextarea = screen.getByLabelText(/Files · the package root, inline/i);
      const submitBtn = screen.getByRole('button', { name: /Create package/i });

      await userEvent.type(nameInput, 'pending-pkg');
      fireEvent.change(filesTextarea, { target: { value: '--- path: departments/my-dept/main.lua\nreturn {}' } });
      
      // Click submit (spawns mutation async)
      await userEvent.click(submitBtn);

      // Verify button is disabled during submission
      await waitFor(() => {
        expect(submitBtn).toBeDisabled();
      });

      // Resolve mutation
      resolveMutation({ name: 'pending-pkg' });
    });
  });

  describe('Create Success & Query Invalidation', () => {
    it('successfully creates a package (201), refetches list, and fires toast', async () => {
      let getListCallCount = 0;
      let postCallCount = 0;

      // Spy on the toast method
      const toastSpy = vi.spyOn(toaster, 'toast');

      server.use(
        http.get('*/api/v1/packages', () => {
          getListCallCount++;
          if (getListCallCount === 1) {
            return HttpResponse.json(['pkg-1']);
          } else {
            return HttpResponse.json(['pkg-1', 'new-pkg']);
          }
        }),
        http.get('*/api/v1/packages/pkg-1', () => {
          return HttpResponse.json({
            name: 'pkg-1',
            files: [],
            composed_deps: [],
            created_at: '',
            updated_at: '',
          });
        }),
        http.get('*/api/v1/packages/new-pkg', () => {
          return HttpResponse.json({
            name: 'new-pkg',
            files: [],
            composed_deps: [],
            created_at: '',
            updated_at: '',
          });
        }),
        http.post('*/api/v1/packages', async ({ request }) => {
          postCallCount++;
          const body = (await request.json()) as { name: string; files: { path: string; content: string }[] };
          expect(body.name).toBe('new-pkg');
          expect(body.files?.length).toBe(1);
          expect(body.files?.[0]?.path).toBe('departments/new-dept/main.lua');
          return HttpResponse.json({ name: 'new-pkg' }, { status: 201 });
        })
      );

      render(<PackagesScreen />, { wrapper: createTestWrapper() });

      // Initially loads pkg-1
      await waitFor(() => expect(screen.getByText('pkg-1')).toBeInTheDocument());
      expect(getListCallCount).toBe(1);

      // Open the modal
      const addBtn = screen.getByRole('button', { name: /^\+ Add package$/i });
      await userEvent.click(addBtn);

      // Fill out form
      const nameInput = screen.getByLabelText(/Name · unique, create-only/i);
      const filesTextarea = screen.getByLabelText(/Files · the package root, inline/i);
      const submitBtn = screen.getByRole('button', { name: /Create package/i });

      await userEvent.type(nameInput, 'new-pkg');
      fireEvent.change(filesTextarea, { target: { value: '--- path: departments/new-dept/main.lua\nreturn {}' } });
      await userEvent.click(submitBtn);

      // Modal should submit successfully and list should refetch
      await waitFor(() => expect(postCallCount).toBe(1));
      await waitFor(() => expect(getListCallCount).toBe(2));
      await waitFor(() => expect(screen.getByText('new-pkg')).toBeInTheDocument());

      // Assert toast copy matches spec exactly
      expect(toastSpy).toHaveBeenCalledWith({
        title: 'Created',
        description: 'Created — composes on next session start',
      });
    });
  });
});
