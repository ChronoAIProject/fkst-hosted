import type { Meta, StoryObj } from '@storybook/react-vite';
import {
  Segmented,
  SegmentedList,
  SegmentedTrigger,
  SegmentedContent,
} from './segmented';

const meta: Meta<typeof Segmented> = {
  title: 'Primitives/Segmented',
  component: Segmented,
};

export default meta;
type Story = StoryObj<typeof Segmented>;

export const Default: Story = {
  render: () => (
    <Segmented defaultValue="live">
      <SegmentedList>
        <SegmentedTrigger value="live">Live</SegmentedTrigger>
        <SegmentedTrigger value="1h">1h</SegmentedTrigger>
        <SegmentedTrigger value="24h">24h</SegmentedTrigger>
        <SegmentedTrigger value="7d">7d</SegmentedTrigger>
        <SegmentedTrigger value="30d">30d</SegmentedTrigger>
      </SegmentedList>
      <div className="mt-4 p-4 border border-line bg-raise rounded-panel">
        <SegmentedContent value="live">
          <p className="text-dim text-[14px]">View content showing data as of last poll...</p>
        </SegmentedContent>
        <SegmentedContent value="1h">
          <p className="text-dim text-[14px]">Hourly rollup analytics...</p>
        </SegmentedContent>
        <SegmentedContent value="24h">
          <p className="text-dim text-[14px]">24-hour rollup analytics...</p>
        </SegmentedContent>
        <SegmentedContent value="7d">
          <p className="text-dim text-[14px]">7-day rollup analytics...</p>
        </SegmentedContent>
        <SegmentedContent value="30d">
          <p className="text-dim text-[14px]">30-day rollup analytics...</p>
        </SegmentedContent>
      </div>
    </Segmented>
  ),
};

export const DisabledTrigger: Story = {
  render: () => (
    <Segmented defaultValue="live">
      <SegmentedList>
        <SegmentedTrigger value="live">Live</SegmentedTrigger>
        <SegmentedTrigger value="1h">1h</SegmentedTrigger>
        <SegmentedTrigger value="24h" disabled>24h (Disabled)</SegmentedTrigger>
        <SegmentedTrigger value="7d">7d</SegmentedTrigger>
      </SegmentedList>
      <div className="mt-4 p-4 border border-line bg-raise rounded-panel">
        <SegmentedContent value="live">
          <p className="text-dim text-[14px]">View content showing data as of last poll...</p>
        </SegmentedContent>
        <SegmentedContent value="1h">
          <p className="text-dim text-[14px]">Hourly rollup analytics...</p>
        </SegmentedContent>
      </div>
    </Segmented>
  ),
};
