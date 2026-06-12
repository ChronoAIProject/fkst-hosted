import type { Meta, StoryObj } from '@storybook/react-vite';
import { PackagesView } from './packages-screen';

const meta: Meta<typeof PackagesView> = {
  title: 'Screens/PackagesScreen',
  component: PackagesView,
};

export default meta;
type Story = StoryObj<typeof PackagesView>;

export const Populated: Story = {
  args: {
    isLoadingList: false,
    listError: null,
    selectedPkgName: 'github-devloop',
    packageNames: ['github-proxy', 'consensus', 'autochrono', 'github-devloop'],
    packagesData: {
      'github-proxy': {
        isLoading: false,
        pkg: {
          name: 'github-proxy',
          files: [],
          composed_deps: [],
          created_at: '2026-06-13T00:00:00Z',
          updated_at: '2026-06-13T00:00:00Z',
        },
      },
      'consensus': {
        isLoading: false,
        pkg: {
          name: 'consensus',
          files: [],
          composed_deps: [],
          created_at: '2026-06-13T00:00:00Z',
          updated_at: '2026-06-13T00:00:00Z',
        },
      },
      'autochrono': {
        isLoading: false,
        pkg: {
          name: 'autochrono',
          files: [],
          composed_deps: ['consensus'],
          created_at: '2026-06-13T00:00:00Z',
          updated_at: '2026-06-13T00:00:00Z',
        },
      },
      'github-devloop': {
        isLoading: false,
        pkg: {
          name: 'github-devloop',
          files: [
            { path: 'departments/intake_scan/main.lua', content: '' },
            { path: 'departments/intake_judge/main.lua', content: '' },
            { path: 'departments/implement/main.lua', content: '' },
            { path: 'raisers/github_poll.lua', content: '' },
            { path: 'raisers/intake_poll.lua', content: '' },
          ],
          composed_deps: ['github-proxy', 'consensus'],
          created_at: '2026-06-13T00:00:00Z',
          updated_at: '2026-06-13T00:00:00Z',
        },
      },
    },
  },
};

export const LoadingList: Story = {
  args: {
    isLoadingList: true,
    listError: null,
    packageNames: [],
  },
};

export const LoadingDetails: Story = {
  args: {
    isLoadingList: false,
    listError: null,
    packageNames: ['github-proxy', 'consensus'],
    packagesData: {
      'github-proxy': {
        isLoading: true,
      },
      'consensus': {
        isLoading: true,
      },
    },
  },
};

export const ListError: Story = {
  args: {
    isLoadingList: false,
    listError: 'package store unreachable — unknown',
    packageNames: [],
  },
};

export const EmptyList: Story = {
  args: {
    isLoadingList: false,
    listError: null,
    packageNames: [],
  },
};

export const UnknownStates: Story = {
  args: {
    isLoadingList: false,
    listError: null,
    packageNames: ['unknown-api-package'],
    packagesData: {
      'unknown-api-package': {
        isLoading: false,
        pkg: {
          name: 'unknown-api-package',
          files: [],
          composed_deps: [],
          created_at: '2026-06-13T00:00:00Z',
          updated_at: '2026-06-13T00:00:00Z',
        },
      },
    },
  },
};
