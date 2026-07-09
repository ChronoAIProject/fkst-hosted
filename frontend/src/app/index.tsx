import { createBrowserRouter, Navigate, RouterProvider } from 'react-router-dom';
import { Shell } from './shell';
import { Introduction } from '../pages/introduction';
import { GetStarted } from '../pages/get-started';
import { Dashboard } from '../pages/dashboard';
import { LanguageProvider } from '../i18n';
import { AuthProvider } from '../lib/auth/github-auth';

// Vite injects BASE_URL from `base` at build time: '/' for dev/preview, and
// '/fkst-hosted/' for the GitHub Pages build. Deriving the router basename from
// it keeps in-app links correct under the Pages subpath without hardcoding it.
const basename = import.meta.env.BASE_URL.replace(/\/$/, '') || '/';

// The whole site is static — two content routes under one shell. No data
// providers, no auth gate, no API. The router is created once at module load
// so navigation state survives re-renders.
const router = createBrowserRouter(
  [
    {
      path: '/',
      element: <Shell />,
      children: [
        { path: '', element: <Introduction /> },
        { path: 'get-started', element: <GetStarted /> },
        { path: 'dashboard', element: <Dashboard /> },
        // Unknown paths fall back to the landing page.
        { path: '*', element: <Navigate to="/" replace /> },
      ],
    },
  ],
  {
    basename,
    future: {
      v7_relativeSplatPath: true,
    },
  }
);

export function App() {
  return (
    <LanguageProvider>
      <AuthProvider>
        <RouterProvider router={router} future={{ v7_startTransition: true }} />
      </AuthProvider>
    </LanguageProvider>
  );
}
