import type { Meta, StoryObj } from '@storybook/react-vite';
import { Switch } from './switch';

const meta: Meta<typeof Switch> = {
  title: 'Primitives/Switch',
  component: Switch,
};

export default meta;
type Story = StoryObj<typeof Switch>;

export const Default: Story = {
  render: () => (
    <div className="flex items-center gap-2">
      <Switch id="toggle-1" />
      <label htmlFor="toggle-1" className="text-dim text-[13px] cursor-pointer">Toggle package</label>
    </div>
  ),
};

export const Checked: Story = {
  render: () => (
    <div className="flex items-center gap-2">
      <Switch id="toggle-2" defaultChecked />
      <label htmlFor="toggle-2" className="text-fg text-[13px] cursor-pointer">Toggle package</label>
    </div>
  ),
};

export const Disabled: Story = {
  render: () => (
    <div className="flex items-center gap-2">
      <Switch id="toggle-3" disabled />
      <label htmlFor="toggle-3" className="text-dim text-[13px] opacity-[0.62] cursor-not-allowed">Toggle package (Disabled)</label>
    </div>
  ),
};

export const FocusVisible: Story = {
  render: () => (
    <div className="flex items-center gap-2">
      <Switch id="toggle-4" data-testid="switch-focus" />
      <label htmlFor="toggle-4" className="text-dim text-[13px] cursor-pointer">Toggle package (Focused)</label>
    </div>
  ),
  play: async ({ canvasElement }) => {
    const switchEl = canvasElement.querySelector('[data-testid="switch-focus"]') as HTMLButtonElement;
    if (switchEl) {
      switchEl.focus();
    }
  },
};
