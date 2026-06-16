import type { Meta, StoryObj } from '@storybook/react-vite';
import { Tooltip, TooltipTrigger, TooltipContent, TooltipProvider } from './tooltip';

const meta: Meta<typeof Tooltip> = {
  title: 'Primitives/Tooltip',
  component: Tooltip,
};

export default meta;
type Story = StoryObj<typeof Tooltip>;

export const Default: Story = {
  render: () => (
    <TooltipProvider>
      <div className="p-12 flex justify-center">
        <Tooltip>
          <TooltipTrigger asChild>
            <button className="bg-raise border border-line-2 text-dim hover:text-fg hover:border-faint rounded-control px-4 py-2 transition-colors focus-visible:outline-2 focus-visible:outline-amber focus-visible:outline-offset-2">
              Hover me
            </button>
          </TooltipTrigger>
          <TooltipContent>
            <span>This is tooltip info</span>
          </TooltipContent>
        </Tooltip>
      </div>
    </TooltipProvider>
  ),
};

export const Open: Story = {
  args: {
    defaultOpen: true,
  },
  render: (args) => (
    <TooltipProvider>
      <div className="p-12 flex justify-center">
        <Tooltip {...args}>
          <TooltipTrigger asChild>
            <button className="bg-raise border border-line-2 text-dim hover:text-fg hover:border-faint rounded-control px-4 py-2 transition-colors focus-visible:outline-2 focus-visible:outline-amber focus-visible:outline-offset-2">
              Hover me (Initially Open)
            </button>
          </TooltipTrigger>
          <TooltipContent>
            <span>This is tooltip info (Open)</span>
          </TooltipContent>
        </Tooltip>
      </div>
    </TooltipProvider>
  ),
};
