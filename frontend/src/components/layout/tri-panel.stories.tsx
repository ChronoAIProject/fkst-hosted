import type { Meta, StoryObj } from '@storybook/react-vite';
import { TriPanel, TriPanelCell } from './tri-panel';

const meta: Meta<typeof TriPanel> = {
  title: 'Layout/TriPanel',
  component: TriPanel,
  decorators: [
    (Story) => (
      <div className="p-8 max-w-shell mx-auto bg-bg text-fg min-h-screen">
        <Story />
      </div>
    ),
  ],
};

export default meta;
type Story = StoryObj<typeof TriPanel>;

const SampleTriPanel = () => (
  <TriPanel>
    <TriPanelCell
      dotClassName="bg-faint"
      header="header-label-a"
      title="Title Example A"
      body="Example body content that wraps across lines to verify proper responsive and layout behaviors of the panel."
      tagSlot={
        <span className="px-2 py-0.5 bg-raise-2 text-faint border border-line-2 text-[10.5px] font-mono rounded-chip">
          tag-a
        </span>
      }
    />
    <TriPanelCell
      dotClassName="bg-amber"
      header="header-label-b"
      title="Title Example B"
      body="Example body content that wraps across lines to verify proper responsive and layout behaviors of the panel."
      tagSlot={
        <span className="px-2 py-0.5 bg-[color-mix(in_oklab,var(--amber)_15%,transparent)] text-amber border border-[color-mix(in_oklab,var(--amber)_35%,var(--line))] text-[10.5px] font-mono rounded-chip font-medium">
          tag-b
        </span>
      }
    />
    <TriPanelCell
      dotClassName="bg-green"
      header="header-label-c"
      title="Title Example C"
      body="Example body content that wraps across lines to verify proper responsive and layout behaviors of the panel."
      tagSlot={
        <span className="px-2 py-0.5 bg-[color-mix(in_oklab,var(--green)_15%,transparent)] text-green border border-[color-mix(in_oklab,var(--green)_35%,var(--line))] text-[10.5px] font-mono rounded-chip font-medium">
          tag-c
        </span>
      }
    />
  </TriPanel>
);

export const Default: Story = {
  render: () => <SampleTriPanel />,
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
      <SampleTriPanel />
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
      <SampleTriPanel />
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
      <SampleTriPanel />
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
      <SampleTriPanel />
    </div>
  ),
};
