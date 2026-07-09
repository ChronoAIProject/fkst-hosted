import { useEffect } from 'react';
import { Eyebrow } from '@/components/layout/eyebrow';
import { useContent } from '@/i18n';
import { useAuth } from '@/lib/auth/github-auth';

export function Dashboard() {
  const c = useContent();
  const d = c.dashboard;
  const { configured, isAuthenticated, error, signIn } = useAuth();

  useEffect(() => {
    document.title = d.metaTitle;
  }, [d.metaTitle]);

  return (
    <div className="flex flex-col gap-8 max-w-[880px]">
      <header>
        <Eyebrow>{d.eyebrow}</Eyebrow>
        <h1 className="mt-5 font-display font-bold text-[clamp(28px,4vw,40px)] leading-[1.1] tracking-[-0.02em] text-fg">
          {d.title}
        </h1>
        <p className="mt-5 text-[15px] leading-relaxed text-dim max-w-[68ch]">{d.lede}</p>
      </header>

      {error && (
        <div className="border border-line border-l-2 border-l-red rounded-card bg-[color-mix(in_oklab,var(--raise)_55%,transparent)] px-4 py-3 text-[13px] text-dim">
          {d.authError}
        </div>
      )}

      {!isAuthenticated ? (
        <section className="border border-line rounded-panel bg-raise p-8 max-[600px]:p-5 flex flex-col items-start gap-4">
          <h2 className="font-display font-semibold text-[20px] text-fg">{d.signInTitle}</h2>
          <p className="text-[14px] leading-relaxed text-dim max-w-[56ch]">{d.signInBody}</p>
          {configured ? (
            <button
              type="button"
              onClick={signIn}
              className="font-ui font-semibold text-[13.5px] bg-amber text-amber-ink rounded-control px-5 py-2.5 transition-colors hover:brightness-[1.06] cursor-pointer"
            >
              {c.auth.signIn}
            </button>
          ) : (
            <p className="font-mono text-[12px] text-ghost">{d.notConfigured}</p>
          )}
        </section>
      ) : (
        <section className="border border-line rounded-panel bg-raise p-8 max-[600px]:p-5 flex flex-col gap-2">
          <h2 className="font-display font-semibold text-[18px] text-fg">{d.comingSoonTitle}</h2>
          <p className="text-[14px] leading-relaxed text-dim max-w-[56ch]">{d.comingSoonBody}</p>
        </section>
      )}
    </div>
  );
}
