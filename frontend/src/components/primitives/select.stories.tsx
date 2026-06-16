import type { Meta, StoryObj } from '@storybook/react-vite';
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
} from './select';

const meta: Meta<typeof Select> = {
  title: 'Primitives/Select',
  component: Select,
};

export default meta;
type Story = StoryObj<typeof Select>;

export const Default: Story = {
  render: () => (
    <div className="w-[240px]">
      <Select defaultValue="github-devloop">
        <SelectTrigger>
          <SelectValue placeholder="Select graph..." />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="github-devloop">github-devloop</SelectItem>
          <SelectItem value="substrate-run">substrate-run</SelectItem>
          <SelectItem value="nyx-test">nyx-test</SelectItem>
        </SelectContent>
      </Select>
    </div>
  ),
};

export const Disabled: Story = {
  render: () => (
    <div className="w-[240px]">
      <Select defaultValue="github-devloop" disabled>
        <SelectTrigger>
          <SelectValue placeholder="Select graph..." />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="github-devloop">github-devloop</SelectItem>
          <SelectItem value="substrate-run">substrate-run</SelectItem>
        </SelectContent>
      </Select>
    </div>
  ),
};

export const Open: Story = {
  args: {
    defaultOpen: true,
  },
  render: (args) => (
    <div className="w-[240px] pb-32">
      <Select {...args} defaultValue="github-devloop">
        <SelectTrigger>
          <SelectValue placeholder="Select graph..." />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="github-devloop">github-devloop</SelectItem>
          <SelectItem value="substrate-run">substrate-run</SelectItem>
          <SelectItem value="nyx-test">nyx-test</SelectItem>
        </SelectContent>
      </Select>
    </div>
  ),
};
