import React from 'react';
import ReactDOM from 'react-dom/client';
import { App } from './app';
import { NyxIDProvider, authRequired } from './lib/auth';
import { ConfigError } from './components/config-error';

import './index.css';

const baseUrl = import.meta.env.VITE_NYXID_BASE_URL || '';
const clientId = import.meta.env.VITE_NYXID_CLIENT_ID || '';
const origin = typeof window !== 'undefined' ? window.location.origin : 'http://localhost';
const redirectUri = import.meta.env.VITE_NYXID_REDIRECT_URI || `${origin}/auth/callback`;
const scope = 'openid profile email';

const hasConfigError = authRequired() === true && (!baseUrl || !clientId);

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    {hasConfigError ? (
      <ConfigError />
    ) : (
      <NyxIDProvider
        baseUrl={baseUrl}
        clientId={clientId}
        redirectUri={redirectUri}
        scope={scope}
      >
        <App />
      </NyxIDProvider>
    )}
  </React.StrictMode>
);
