import { useState } from 'react';
import { useAuthSession } from '../../lib/auth';

export function SignInGate() {
  const { login } = useAuthSession();
  const [loading, setLoading] = useState(false);

  const handleLogin = async () => {
    setLoading(true);
    try {
      await login();
    } catch (err) {
      console.error('Login redirection failed:', err);
      setLoading(false);
    }
  };

  return (
    <div className="min-h-screen flex items-center justify-center bg-bg text-fg font-ui px-4 select-none">
      <div className="max-w-md w-full bg-raise border border-line rounded-xl p-8 shadow-modal-seat relative overflow-hidden backdrop-blur-md bg-opacity-70">
        {/* Subtle decorative line at the top */}
        <div className="absolute top-0 left-0 right-0 h-1 bg-amber" />

        <div className="flex flex-col items-center mb-6">
          <div className="font-display font-bold text-[32px] tracking-[0.01em] text-fg no-underline leading-none mb-4 relative select-none">
            F
            <span className="relative inline-block">
              K
              <span
                style={{ boxShadow: '0 0 0 0.05em var(--bg)' }}
                className="absolute left-[0.04em] top-[0.36em] w-[0.205em] h-[0.205em] rounded-full bg-amber"
                aria-hidden="true"
              />
            </span>
            ST
          </div>
          <p className="text-xs text-ghost font-mono">
            ChronoAI fkst-hosted Auth
          </p>
        </div>

        <div className="space-y-6">
          <div className="text-center space-y-2">
            <h1 className="font-display font-bold text-lg tracking-[0.01em] text-fg">
              Authentication Required
            </h1>
            <p className="text-sm text-dim leading-relaxed">
              This deployment requires a verified identity to manage goals, sessions, and packages.
            </p>
          </div>

          <button
            onClick={handleLogin}
            disabled={loading}
            className="w-full font-ui font-semibold text-[13.5px] bg-amber text-amber-ink rounded-control py-3 flex items-center justify-center gap-2 transition-all hover:brightness-[1.06] cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed focus-visible:outline-2 focus-visible:outline-amber focus-visible:outline-offset-2"
          >
            {loading ? (
              <span>Redirecting to NyxID...</span>
            ) : (
              <span>Sign in with NyxID</span>
            )}
          </button>

          <div className="pt-2 text-[11px] text-ghost leading-relaxed text-center font-mono">
            Identity and access brokered securely by NyxID
          </div>
        </div>
      </div>
    </div>
  );
}
