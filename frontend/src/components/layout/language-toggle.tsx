import { cn } from '@/lib/utils';
import { useLang, useContent } from '@/i18n';
import type { Lang } from '@/i18n';

export function LanguageToggle({ className }: { className?: string }) {
  const { lang, setLang } = useLang();
  const c = useContent();
  const options: { value: Lang; label: string }[] = [
    { value: 'en', label: c.toggle.en },
    { value: 'zh', label: c.toggle.zh },
  ];

  return (
    <div
      role="group"
      aria-label={c.toggle.aria}
      className={cn(
        // Frosted segmented control: gradient hairline over a blurred glass fill.
        'inline-flex items-center rounded-control grad-border bg-glass backdrop-blur-glass p-0.5 shadow-1',
        className
      )}
    >
      {options.map((o) => {
        const active = o.value === lang;
        return (
          <button
            key={o.value}
            type="button"
            onClick={() => setLang(o.value)}
            aria-pressed={active}
            className={cn(
              'font-mono text-[11.5px] px-2 py-[3px] rounded-chip cursor-pointer',
              'transition-[color,background-color,box-shadow] duration-200',
              // Active pill wears the amber accent fill + a soft glow; inactive
              // stays quiet and warms toward amber on hover.
              active
                ? 'bg-grad-accent text-amber-ink shadow-glow-amber'
                : 'text-faint hover:text-amber'
            )}
          >
            {o.label}
          </button>
        );
      })}
    </div>
  );
}
