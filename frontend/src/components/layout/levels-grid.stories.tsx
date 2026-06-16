import type { Meta, StoryObj } from '@storybook/react-vite';
import { LevelsGrid, LevelsGridCell } from './levels-grid';

const meta: Meta<typeof LevelsGrid> = {
  title: 'Layout/LevelsGrid',
  component: LevelsGrid,
  decorators: [
    (Story) => (
      <div className="p-8 max-w-shell mx-auto bg-bg text-fg min-h-screen">
        <Story />
      </div>
    ),
  ],
};

export default meta;
type Story = StoryObj<typeof LevelsGrid>;

const SampleGrid = () => (
  <LevelsGrid>
    <LevelsGridCell
      eyebrow="kicker-label-a"
      value="value-text-a"
      description="Example description text that wraps across lines to verify proper responsive and layout behaviors of the cell."
    />
    <LevelsGridCell
      eyebrow="kicker-label-b"
      value="value-text-b"
      description="Example description text that wraps across lines to verify proper responsive and layout behaviors of the cell."
    />
    <LevelsGridCell
      eyebrow="kicker-label-c"
      value="value-text-c"
      description="Example description text that wraps across lines to verify proper responsive and layout behaviors of the cell."
    />
  </LevelsGrid>
);

export const Default: Story = {
  render: () => <SampleGrid />,
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
      <SampleGrid />
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
      <SampleGrid />
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
      <SampleGrid />
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
      <SampleGrid />
    </div>
  ),
};
