import { lazy, Suspense } from 'react';
import { createBrowserRouter, Navigate, RouterProvider } from 'react-router-dom';
import { Shell } from './shell';
import { Introduction } from '../pages/introduction';
import { GetStarted } from '../pages/get-started';
import { LanguageProvider, useContent } from '../i18n';
import { AuthProvider } from '../lib/auth/github-auth';

// The dashboard carries the canvas stack (React Flow, recharts,
// framer-motion) — lazy-load it so the docs pages stay a light bundle.
const Dashboard = lazy(() =>
  import('../pages/dashboard').then((m) => ({ default: m.Dashboard }))
);

/** Route-level skeleton shown while the lazy dashboard chunk downloads —
 *  page-shaped shimmer blocks (index.css vocabulary) so the content area
 *  never flashes blank on a slow connection. Exported for its unit test. */
export function DashboardFallback() {
  const d = useContent().dashboard;
  return (
    <div
      role="status"
      aria-label={d.loading}
      data-testid="dashboard-route-skeleton"
      className="flex flex-col gap-6"
    >
      <div className="anim-shimmer rounded-chip h-4 w-28" />
      <div className="anim-shimmer rounded-card h-10 w-2/3 max-w-[520px]" />
      <div className="anim-shimmer rounded-chip h-4 w-1/2 max-w-[400px]" />
      <div className="anim-shimmer rounded-panel h-[440px]" />
    </div>
  );
}

// Vite injects BASE_URL from `base` at build time: '/' for dev/preview, and
// '/fkst-hosted/' for the GitHub Pages build. Deriving the router basename from
// it keeps in-app links correct under the Pages subpath without hardcoding it.
const basename = import.meta.env.BASE_URL.replace(/\/$/, '') || '/';

// Two static content routes plus the lazy dashboard under one shell. The
// router is created once at module load so navigation state survives
// re-renders.
const router = createBrowserRouter(
  [
    {
      path: '/',
      element: <Shell />,
      children: [
        { path: '', element: <Introduction /> },
        { path: 'get-started', element: <GetStarted /> },
        {
          path: 'dashboard',
          element: (
            <Suspense fallback={<DashboardFallback />}>
              <Dashboard />
            </Suspense>
          ),
        },
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
