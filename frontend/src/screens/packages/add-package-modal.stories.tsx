import type { Meta, StoryObj } from '@storybook/react-vite';
import { userEvent, within } from 'storybook/test';
import { useEffect } from 'react';
import { AddPackageModal } from './add-package-modal';

const meta: Meta<typeof AddPackageModal> = {
  title: 'Screens/AddPackageModal',
  component: AddPackageModal,
  args: {
    isOpen: true,
    onOpenChange: () => {},
  },
};

export default meta;
type Story = StoryObj<typeof AddPackageModal>;

// Standard empty form dialog
export const Default: Story = {};

// Automated story using play function to click submit and trigger Zod validation errors
export const ValidationErrors: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const submitBtn = canvas.getByRole('button', { name: /Create package/i });
    await userEvent.click(submitBtn);
  },
};

// Automated story simulating server 409 conflict error when trying to create duplicate package
export const Server409Conflict: Story = {
  decorators: [
    (Story) => {
      useEffect(() => {
        const originalFetch = window.fetch;
        window.fetch = async (url, options) => {
          if (typeof url === 'string' && url.includes('/api/v1/packages')) {
            return new Response(
              JSON.stringify({
                error: 'conflict',
                message: 'name already exists (a revision is a new name)',
              }),
              {
                status: 409,
                headers: { 'Content-Type': 'application/json' },
              }
            );
          }
          return originalFetch(url, options);
        };
        return () => {
          window.fetch = originalFetch;
        };
      }, []);

      return <Story />;
    },
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const nameInput = canvas.getByLabelText(/Name · unique, create-only/i);
    const filesTextarea = canvas.getByLabelText(/Files · the package root, inline/i);
    const submitBtn = canvas.getByRole('button', { name: /Create package/i });

    // Enter valid details and click submit to trigger the mock 409 error
    await userEvent.type(nameInput, 'duplicate-pkg');
    await userEvent.type(
      filesTextarea,
      '--- path: departments/my-dept/main.lua\nprint("hello")'
    );
    await userEvent.click(submitBtn);
  },
};
