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
        'inline-flex items-center rounded-control border border-line bg-raise p-0.5',
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
              'font-mono text-[11.5px] px-2 py-[3px] rounded-chip transition-colors cursor-pointer',
              active ? 'bg-raise-2 text-fg' : 'text-faint hover:text-dim'
            )}
          >
            {o.label}
          </button>
        );
      })}
    </div>
  );
}
