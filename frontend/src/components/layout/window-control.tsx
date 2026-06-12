import { cn } from '@/lib/utils';

export interface WindowControlProps {
  value: string;
  onChange: (value: string) => void;
  options?: string[];
  className?: string;
}

const DEFAULT_OPTIONS = ['Live', '1h', '24h', '7d', '30d'];

export function WindowControl({
  value,
  onChange,
  options = DEFAULT_OPTIONS,
  className,
}: WindowControlProps) {
  return (
    <div
      className={cn(
        'bg-raise border border-line rounded-control p-[2px] inline-flex items-center select-none',
        className
      )}
      role="group"
      aria-label="Time window selector"
    >
      {options.map((option) => {
        const isActive = option === value;
        return (
          <button
            key={option}
            type="button"
            onClick={() => onChange(option)}
            className={cn(
              'py-[5px] px-[13px] text-[13px] font-medium rounded-chip transition-colors cursor-pointer outline-none focus-visible:outline-2 focus-visible:outline-amber focus-visible:outline-offset-2',
              isActive
                ? 'bg-amber text-amber-ink font-semibold'
                : 'text-faint hover:text-dim hover:bg-raise-2'
            )}
            aria-pressed={isActive}
          >
            {option}
          </button>
        );
      })}
    </div>
  );
}
