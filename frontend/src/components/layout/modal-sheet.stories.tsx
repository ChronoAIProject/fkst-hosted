import type { Meta, StoryObj } from '@storybook/react-vite';
import { ModalSheet } from './modal-sheet';

const meta: Meta<typeof ModalSheet> = {
  title: 'Layout/ModalSheet',
  component: ModalSheet,
  decorators: [
    (Story) => (
      <div className="p-8 max-w-shell mx-auto bg-bg text-fg min-h-screen">
        <Story />
      </div>
    ),
  ],
};

export default meta;
type Story = StoryObj<typeof ModalSheet>;

const CloseButton = () => (
  <button
    type="button"
    className="p-1 rounded-chip text-faint hover:text-fg hover:bg-raise-2 transition-colors cursor-pointer outline-none focus-visible:outline-2 focus-visible:outline-amber"
    aria-label="Close"
  >
    <svg
      className="w-4 h-4"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      viewBox="0 0 24 24"
    >
      <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
    </svg>
  </button>
);

const ActionLeft = () => (
  <span className="font-mono text-[11px] text-faint bg-raise border border-line px-2 py-0.5 rounded-chip">
    example-status
  </span>
);

const ActionButtons = () => (
  <>
    <button
      type="button"
      className="px-3 py-1.5 bg-raise border border-line-2 text-dim hover:text-fg hover:border-faint rounded-control text-xs font-medium transition-colors cursor-pointer outline-none focus-visible:outline-2 focus-visible:outline-amber"
    >
      Cancel
    </button>
    <button
      type="button"
      className="px-3 py-1.5 bg-amber text-amber-ink hover:brightness-[1.06] rounded-control text-xs font-semibold transition-colors cursor-pointer outline-none focus-visible:outline-2 focus-visible:outline-amber"
    >
      Confirm
    </button>
  </>
);

const BodyContent = () => (
  <div className="space-y-4">
    <p className="text-dim leading-relaxed">
      Example description text inside the modal sheet layout. This text is meant to wrap properly and align with the panel specifications.
    </p>
    <div className="p-4 border border-line bg-raise rounded-card space-y-2">
      <div className="text-xs text-ghost font-mono">example-label</div>
      <div className="text-sm font-semibold font-mono text-fg">
        example-identifier-or-value
      </div>
    </div>
  </div>
);

export const SheetOnly: Story = {
  render: () => (
    <ModalSheet
      title="example-heading-text"
      meta="example-meta-text · detail-item"
      closeButtonSlot={<CloseButton />}
      actionLeftSlot={<ActionLeft />}
      actionButtonsSlot={<ActionButtons />}
    >
      <BodyContent />
    </ModalSheet>
  ),
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
      <ModalSheet
        title="example-heading-text"
        meta="example-meta-text · detail-item"
        closeButtonSlot={<CloseButton />}
        actionLeftSlot={<ActionLeft />}
        actionButtonsSlot={<ActionButtons />}
      >
        <BodyContent />
      </ModalSheet>
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
      <ModalSheet
        title="example-heading-text"
        meta="example-meta-text · detail-item"
        closeButtonSlot={<CloseButton />}
        actionLeftSlot={<ActionLeft />}
        actionButtonsSlot={<ActionButtons />}
      >
        <BodyContent />
      </ModalSheet>
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
      <ModalSheet
        title="example-heading-text"
        meta="example-meta-text · detail-item"
        closeButtonSlot={<CloseButton />}
        actionLeftSlot={<ActionLeft />}
        actionButtonsSlot={<ActionButtons />}
      >
        <BodyContent />
      </ModalSheet>
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
      <ModalSheet
        title="example-heading-text"
        meta="example-meta-text · detail-item"
        closeButtonSlot={<CloseButton />}
        actionLeftSlot={<ActionLeft />}
        actionButtonsSlot={<ActionButtons />}
      >
        <BodyContent />
      </ModalSheet>
    </div>
  ),
};
