import { cn } from '@/lib/utils';

export interface ViewSwitchProps {
  value: 'pipeline' | 'board';
  onChange: (value: 'pipeline' | 'board') => void;
  className?: string;
}

const PipeIcon = () => (
  <svg
    viewBox="0 0 16 16"
    fill="none"
    stroke="currentColor"
    strokeWidth="1.5"
    strokeLinecap="round"
    strokeLinejoin="round"
    className="w-4 h-4"
  >
    <circle cx="3.5" cy="8" r="1.5" />
    <circle cx="8" cy="8" r="1.5" />
    <circle cx="12.5" cy="8" r="1.5" />
    <path d="M5 8h1.5M9.5 8h1.5" />
  </svg>
);

const BoardIcon = () => (
  <svg
    viewBox="0 0 16 16"
    fill="none"
    stroke="currentColor"
    strokeWidth="1.5"
    strokeLinecap="round"
    strokeLinejoin="round"
    className="w-4 h-4"
  >
    <rect x="2" y="2" width="3.2" height="12" rx="0.5" />
    <rect x="6.4" y="2" width="3.2" height="12" rx="0.5" />
    <rect x="10.8" y="2" width="3.2" height="12" rx="0.5" />
  </svg>
);

export function ViewSwitch({ value, onChange, className }: ViewSwitchProps) {
  return (
    <div
      className={cn(
        'bg-raise border border-line rounded-control p-[2px] inline-flex items-center select-none',
        className
      )}
      role="group"
      aria-label="View switch toggle"
    >
      <button
        type="button"
        onClick={() => onChange('pipeline')}
        className={cn(
          'py-[5px] px-[13px] rounded-chip transition-colors cursor-pointer outline-none focus-visible:outline-2 focus-visible:outline-amber focus-visible:outline-offset-2',
          value === 'pipeline'
            ? 'text-amber bg-raise-2'
            : 'text-faint hover:text-dim hover:bg-raise-2'
        )}
        aria-label="Pipeline view"
        aria-pressed={value === 'pipeline'}
      >
        <PipeIcon />
      </button>
      <button
        type="button"
        onClick={() => onChange('board')}
        className={cn(
          'py-[5px] px-[13px] rounded-chip transition-colors cursor-pointer outline-none focus-visible:outline-2 focus-visible:outline-amber focus-visible:outline-offset-2',
          value === 'board'
            ? 'text-amber bg-raise-2'
            : 'text-faint hover:text-dim hover:bg-raise-2'
        )}
        aria-label="Board view"
        aria-pressed={value === 'board'}
      >
        <BoardIcon />
      </button>
    </div>
  );
}
