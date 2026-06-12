import type { Meta, StoryObj } from '@storybook/react-vite';
// eslint-disable-next-line storybook/use-storybook-testing-library
import userEvent from '@testing-library/user-event';
import {
  Dialog,
  DialogTrigger,
  DialogContent,
  DialogTitle,
  DialogDescription,
} from './dialog';

const meta: Meta<typeof Dialog> = {
  title: 'Primitives/Dialog',
  component: Dialog,
};

export default meta;
type Story = StoryObj<typeof Dialog>;

export const Default: Story = {
  render: () => (
    <Dialog>
      <DialogTrigger asChild>
        <button className="bg-raise border border-line-2 text-dim hover:text-fg hover:border-faint rounded-control px-4 py-2 transition-colors focus-visible:outline-2 focus-visible:outline-amber focus-visible:outline-offset-2">
          Open Dialog
        </button>
      </DialogTrigger>
      <DialogContent>
        <DialogTitle>Issue Detail</DialogTitle>
        <DialogDescription>
          This is a token-skinned Radix Dialog primitive. It supports focus trapping, backdrop closing, and Esc key closing.
        </DialogDescription>
        <div className="mt-6 flex justify-end gap-2">
          <button className="bg-raise border border-line-2 text-dim hover:text-fg hover:border-faint rounded-control px-4 py-2 transition-colors">
            Cancel
          </button>
          <button className="bg-amber text-amber-ink hover:brightness-[1.06] rounded-control px-4 py-2 font-semibold transition-colors">
            Confirm
          </button>
        </div>
      </DialogContent>
    </Dialog>
  ),
};

export const Open: Story = {
  args: {
    defaultOpen: true,
  },
  render: (args) => (
    <Dialog {...args}>
      <DialogTrigger asChild>
        <button className="bg-raise border border-line-2 text-dim hover:text-fg hover:border-faint rounded-control px-4 py-2 transition-colors focus-visible:outline-2 focus-visible:outline-amber focus-visible:outline-offset-2">
          Open Dialog (Initially Open)
        </button>
      </DialogTrigger>
      <DialogContent>
        <DialogTitle>Initially Open Dialog</DialogTitle>
        <DialogDescription>
          This story renders the Dialog content in an open state by default.
        </DialogDescription>
      </DialogContent>
    </Dialog>
  ),
};

export const OpenViaPlay: Story = {
  render: () => (
    <Dialog>
      <DialogTrigger asChild>
        <button data-testid="trigger" className="bg-raise border border-line-2 text-dim hover:text-fg hover:border-faint rounded-control px-4 py-2 transition-colors focus-visible:outline-2 focus-visible:outline-amber focus-visible:outline-offset-2">
          Open Dialog (Play Function)
        </button>
      </DialogTrigger>
      <DialogContent>
        <DialogTitle>Play Function Dialog</DialogTitle>
        <DialogDescription>
          This dialog was opened automatically using a Storybook play function.
        </DialogDescription>
      </DialogContent>
    </Dialog>
  ),
  play: async ({ canvasElement }) => {
    const trigger = canvasElement.querySelector('[data-testid="trigger"]') as HTMLButtonElement;
    if (trigger) {
      await userEvent.click(trigger);
    }
  },
};
