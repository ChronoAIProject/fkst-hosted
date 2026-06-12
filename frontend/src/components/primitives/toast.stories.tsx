import type { Meta, StoryObj } from '@storybook/react-vite';
import * as React from 'react';
import { Toast, ToastProvider, ToastViewport, ToastTitle, ToastDescription, ToastAction, ToastClose } from './toast';
import { Toaster, toast } from './toaster';

const meta: Meta<typeof Toast> = {
  title: 'Primitives/Toast',
  component: Toast,
};

export default meta;
type Story = StoryObj<typeof Toast>;

export const Default: Story = {
  render: () => {
    const [open, setOpen] = React.useState(false);

    return (
      <ToastProvider>
        <button
          onClick={() => setOpen(true)}
          className="bg-amber text-amber-ink hover:brightness-[1.06] rounded-control px-4 py-2 font-medium transition-colors focus-visible:outline-2 focus-visible:outline-amber focus-visible:outline-offset-2"
        >
          Show Toast
        </button>

        <Toast open={open} onOpenChange={setOpen}>
          <div className="grid gap-1">
            <ToastTitle>Example notification</ToastTitle>
            <ToastDescription>Your settings have been saved.</ToastDescription>
          </div>
          <ToastAction altText="Undo action" asChild>
            <button
              onClick={() => setOpen(false)}
              className="bg-raise border border-line-2 text-dim hover:text-fg hover:border-faint rounded-control px-3 py-1.5 transition-colors text-[12.5px] cursor-pointer"
            >
              Undo
            </button>
          </ToastAction>
          <ToastClose />
        </Toast>

        <ToastViewport />
      </ToastProvider>
    );
  },
};

export const Fired: Story = {
  render: () => {
    const triggerToast = () => {
      toast({
        title: 'Saved',
        description: 'Changes applied successfully.',
        action: {
          text: 'Dismiss',
          onClick: () => {
            // Dismiss action
          },
        },
      });
    };

    return (
      <div className="p-4">
        <button
          onClick={triggerToast}
          className="bg-amber text-amber-ink hover:brightness-[1.06] rounded-control px-4 py-2 font-medium transition-colors focus-visible:outline-2 focus-visible:outline-amber focus-visible:outline-offset-2"
        >
          Trigger Global Toast
        </button>
        <Toaster />
      </div>
    );
  },
};
