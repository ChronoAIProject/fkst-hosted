import type { Meta, StoryObj } from '@storybook/react-vite';
import { Overview } from './overview';
import {
  overviewPipelineFixture,
  overviewBoardFixture,
} from '@/fixtures/overview';

const meta: Meta<typeof Overview> = {
  title: 'Mock / Overview',
  component: Overview,
  decorators: [
    (Story) => (
      <div className="flex flex-col min-h-screen bg-bg text-fg">
        <div
          style={{
            background: 'color-mix(in oklab, var(--gold) 12%, transparent)',
            borderBottom: '1px solid color-mix(in oklab, var(--gold) 35%, var(--line))',
            color: 'var(--gold)',
            fontSize: '12.5px',
            padding: '6px 16px',
            fontFamily: 'var(--mono)',
            userSelect: 'none',
            textAlign: 'center',
          }}
        >
          MOCK DATA — simulates the GitHub plane post-NyxID; live routes render honest shells
        </div>
        <div className="p-6 flex-1">
          <Story />
        </div>
      </div>
    ),
  ],
};

export default meta;
type Story = StoryObj<typeof Overview>;

export const PipelinePopulated: Story = {
  args: {
    ...overviewPipelineFixture,
    initialView: 'pipeline',
    initialWindow: '24h',
  },
  parameters: {
    docs: {
      description: {
        story: 'Mock data — represents the GitHub plane after NyxID lands; live routes render honest empty shells until then.',
      },
    },
  },
};

export const BoardPopulated: Story = {
  args: {
    ...overviewBoardFixture,
    initialView: 'board',
    initialWindow: '24h',
  },
  parameters: {
    docs: {
      description: {
        story: 'Mock data — represents the GitHub plane after NyxID lands; live routes render honest empty shells until then.',
      },
    },
  },
};

export const NeedsYouEmpty: Story = {
  args: {
    ...overviewPipelineFixture,
    needsYou: [],
    initialView: 'pipeline',
    initialWindow: '24h',
  },
  parameters: {
    docs: {
      description: {
        story: 'Mock data — represents the GitHub plane after NyxID lands; live routes render honest empty shells until then.',
      },
    },
  },
};

export const WindowedLive: Story = {
  args: {
    ...overviewPipelineFixture,
    vitals: {
      ...overviewPipelineFixture.vitals,
      inFlight: 5,
      merged24h: 1,
      deadEnded: 2, // 2 matches the terminal blocked row present in needs-you
      throughput: 'unknown',
      medianReviewTime: 'unknown',
      windowStart: '04:00 Jun 13',
      windowEnd: '04:05 Jun 13',
    },
    initialView: 'pipeline',
    initialWindow: 'Live',
  },
  parameters: {
    docs: {
      description: {
        story: 'Mock data — represents the GitHub plane after NyxID lands; live routes render honest empty shells until then.',
      },
    },
  },
};

export const Windowed7d: Story = {
  name: 'Windowed 7d',
  args: {
    ...overviewPipelineFixture,
    vitals: {
      ...overviewPipelineFixture.vitals,
      merged24h: 55, // Plausible merged count over 24h given throughput (~3/h -> <=72/24h)
      deadEnded: 10,
      throughput: '~3/h',
      medianReviewTime: '45m',
      windowStart: 'Jun 6',
      windowEnd: 'Jun 13',
    },
    initialView: 'pipeline',
    initialWindow: '7d',
  },
  parameters: {
    docs: {
      description: {
        story: 'Mock data — represents the GitHub plane after NyxID lands; live routes render honest empty shells until then.',
      },
    },
  },
};
