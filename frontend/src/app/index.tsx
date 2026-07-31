import { lazy, Suspense } from 'react';
import { createBrowserRouter, RouterProvider } from 'react-router-dom';
import { Shell } from './shell';
import { Introduction } from '../pages/introduction';
import { GetStarted } from '../pages/get-started';
import { NotFound } from '../pages/not-found';
import { ErrorBoundary, RouteErrorElement } from '../components/ui/error-boundary';
import { ToastProvider, Toaster } from '../components/ui/toast';
import { TourProvider } from '../components/tour/tour-context';
import { LanguageProvider, useContent } from '../i18n';
import { AuthProvider } from '../lib/auth/github-auth';
import { BroaderOAuthProvider } from '../lib/auth/broader-oauth';

// The dashboard carries the canvas stack (React Flow, recharts,
// framer-motion) — lazy-load it so the docs pages stay a light bundle.
const Dashboard = lazy(() =>
  import('../pages/dashboard').then((m) => ({ default: m.Dashboard }))
);

// The operations workspace carries its own tables, filter toolbar, and icon set
// and is only ever opened deliberately — lazy-load it so neither the docs pages
// nor the dashboard pay for it.
const OperationsPage = lazy(() =>
  import('../pages/operations').then((m) => ({ default: m.Operations }))
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

/** Route-level skeleton for the lazy operations chunk. Shaped like the page it
 *  replaces — a toolbar strip above one tall table region — so the fixed-height
 *  layout does not jump when the real workspace mounts. Exported for its test. */
export function OperationsFallback() {
  const t = useContent().operations;
  return (
    <div
      role="status"
      aria-label={t.loading}
      data-testid="operations-route-skeleton"
      className="h-full flex flex-col gap-3"
    >
      <div className="anim-shimmer rounded-chip h-6 w-56 flex-none" />
      <div className="anim-shimmer rounded-control h-8 w-full max-w-[720px] flex-none" />
      <div className="anim-shimmer rounded-panel flex-1 min-h-[240px]" />
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
      // Shell itself throwing has no chrome left to preserve, so fall back to
      // the full-area error view here.
      errorElement: <RouteErrorElement />,
      children: [
        {
          // Pathless layout route: React Router renders its implicit <Outlet />
          // into the Shell, and swaps in this errorElement *in that slot* when a
          // page throws — so the topbar/footer stay put and the fallback shows
          // in-shell rather than blanking the whole app.
          errorElement: <RouteErrorElement />,
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
            {
              path: 'operations',
              element: (
                <Suspense fallback={<OperationsFallback />}>
                  <OperationsPage />
                </Suspense>
              ),
            },
            // Unknown paths render a real 404 that names the missing path,
            // instead of silently redirecting to the landing page.
            { path: '*', element: <NotFound /> },
          ],
        },
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

/** The single global Toaster, localized. Split out so it can read `useContent`
 *  inside the providers while `App` stays a plain provider-mount tree. */
function ShellToaster() {
  const s = useContent().shell;
  return <Toaster dismissLabel={s.toastDismiss} />;
}

export function App() {
  return (
    <LanguageProvider>
      {/* Outermost boundary catches render throws the router's errorElement
          cannot reach (providers, the router host, the toaster). */}
      <ErrorBoundary>
        <AuthProvider>
          {/* Captures the broader-visibility token from the return-redirect
              fragment (#broader_token) app-wide, so it lands regardless of
              which route the OAuth flow returns to — mirroring AuthProvider. */}
          <BroaderOAuthProvider>
            {/* ToastProvider wraps the whole router tree so any page can raise
                notices via useToast(); the one Toaster is mounted alongside.
                TourProvider wraps the RouterProvider tree so every route can
                useTour() (the <TourOverlay/> itself is mounted inside Shell, the
                router root, so the finish step's react-router Link resolves). */}
            <ToastProvider>
              <TourProvider>
                <RouterProvider router={router} future={{ v7_startTransition: true }} />
              </TourProvider>
              <ShellToaster />
            </ToastProvider>
          </BroaderOAuthProvider>
        </AuthProvider>
      </ErrorBoundary>
    </LanguageProvider>
  );
}
