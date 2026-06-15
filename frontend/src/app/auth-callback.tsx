import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useAuthSession } from '../lib/auth';

export function AuthCallback() {
  const { handleRedirectCallback } = useAuthSession();
  const navigate = useNavigate();
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    handleRedirectCallback(window.location.href)
      .then(() => {
        if (active) {
          navigate('/', { replace: true });
        }
      })
      .catch((err) => {
        // Scrub URL: remove code, state, error, error_description
        try {
          const url = new URL(window.location.href);
          url.searchParams.delete('code');
          url.searchParams.delete('state');
          url.searchParams.delete('error');
          url.searchParams.delete('error_description');
          window.history.replaceState({}, '', url.pathname + url.search + url.hash);
        } catch (e) {
          console.error('Failed to scrub URL:', e);
        }

        // Clear pending PKCE storage key
        try {
          const clientId = import.meta.env.VITE_NYXID_CLIENT_ID || '';
          if (clientId) {
            localStorage.removeItem(`nyxid:pending:${clientId}`);
          }
        } catch (e) {
          console.error('Failed to clear PKCE state:', e);
        }

        if (active) {
          setError(err instanceof Error ? err.message : String(err));
        }
      });

    return () => {
      active = false;
    };
  }, [handleRedirectCallback, navigate]);

  if (error) {
    return (
      <div className="min-h-screen flex items-center justify-center bg-bg text-fg px-4">
        <div className="max-w-md w-full bg-raise border border-red/30 rounded-lg p-6 shadow-modal-seat">
          <h1 className="text-red font-bold text-lg mb-2">Authentication Failed</h1>
          <p className="text-dim text-sm mb-4">{error}</p>
          <button
            onClick={() => navigate('/overview', { replace: true })}
            className="w-full py-2 bg-red hover:brightness-110 text-fg font-semibold rounded transition-colors text-sm cursor-pointer"
          >
            Return to Overview
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen flex flex-col items-center justify-center bg-bg text-fg">
      <div className="flex flex-col items-center gap-3">
        <div className="w-8 h-8 border-2 border-amber border-t-transparent rounded-full" />
        <p className="text-sm text-dim font-mono">Completing login...</p>
      </div>
    </div>
  );
}
