import type { Meta, StoryObj } from '@storybook/react-vite';
import { HairlineList, HairlineRow } from './hairline-list';

const meta: Meta<typeof HairlineList> = {
  title: 'Layout/HairlineList',
  component: HairlineList,
  decorators: [
    (Story) => (
      <div className="p-8 max-w-shell mx-auto bg-bg text-fg min-h-screen">
        <Story />
      </div>
    ),
  ],
};

export default meta;
type Story = StoryObj<typeof HairlineList>;

const SampleList = () => (
  <HairlineList>
    <HairlineRow
      leftContent={
        <div className="flex flex-col min-w-0">
          <span className="font-mono text-[12.5px] text-fg font-semibold truncate">
            row-item-a
          </span>
          <span className="text-body text-dim mt-0.5 truncate">
            Example description text that wraps or truncates depending on viewport constraints
          </span>
        </div>
      }
      rightContent={
        <button className="px-3 py-1.5 bg-raise-2 border border-line-2 text-dim hover:text-fg hover:border-faint rounded-control text-xs font-medium transition-colors">
          Action
        </button>
      }
    />
    <HairlineRow
      leftContent={
        <div className="flex flex-col min-w-0">
          <span className="font-mono text-[12.5px] text-fg font-semibold truncate">
            row-item-b
          </span>
          <span className="text-body text-dim mt-0.5 truncate">
            Example description text that wraps or truncates depending on viewport constraints
          </span>
        </div>
      }
      rightContent={
        <button className="px-3 py-1.5 bg-raise-2 border border-line-2 text-dim hover:text-fg hover:border-faint rounded-control text-xs font-medium transition-colors">
          Action
        </button>
      }
    />
  </HairlineList>
);

export const Default: Story = {
  render: () => <SampleList />,
};

export const Viewport480: Story = {
  parameters: {
    viewport: {
      defaultViewport: 'mobile1',
    },
  },
  render: () => (
    <div className="max-w-[480px] border border-dashed border-line p-4 rounded-panel mx-auto bg-bg">
      <div className="text-xs text-ghost font-mono mb-2">Viewport: 480px</div>
      <SampleList />
    </div>
  ),
};

export const Viewport780: Story = {
  parameters: {
    viewport: {
      defaultViewport: 'tablet',
    },
  },
  render: () => (
    <div className="max-w-[780px] border border-dashed border-line p-4 rounded-panel mx-auto bg-bg">
      <div className="text-xs text-ghost font-mono mb-2">Viewport: 780px</div>
      <SampleList />
    </div>
  ),
};

export const Viewport980: Story = {
  parameters: {
    viewport: {
      defaultViewport: 'laptop',
    },
  },
  render: () => (
    <div className="max-w-[980px] border border-dashed border-line p-4 rounded-panel mx-auto bg-bg">
      <div className="text-xs text-ghost font-mono mb-2">Viewport: 980px</div>
      <SampleList />
    </div>
  ),
};

export const Viewport1440: Story = {
  parameters: {
    viewport: {
      defaultViewport: 'desktop',
    },
  },
  render: () => (
    <div className="max-w-[1440px] border border-dashed border-line p-4 rounded-panel mx-auto bg-bg">
      <div className="text-xs text-ghost font-mono mb-2">Viewport: 1440px</div>
      <SampleList />
    </div>
  ),
};
