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
        </AuthProvider>
      </ErrorBoundary>
    </LanguageProvider>
  );
}
