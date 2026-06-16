import type { Preview } from '@storybook/react-vite';
import '../src/index.css';

const preview: Preview = {
  parameters: {
    controls: {
      matchers: {
        color: /(background|color)$/i,
        date: /Date$/i,
      },
    },
    a11y: {
      test: 'todo',
    },
    // The canvas background is painted by our tailwind/index.css body rule (var(--bg)).
    // Disable Storybook's background switcher to avoid styling conflicts.
    backgrounds: {
      disable: true,
    },
  },
};

export default preview;
