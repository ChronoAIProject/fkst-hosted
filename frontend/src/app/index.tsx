import { createBrowserRouter, Navigate, RouterProvider } from 'react-router-dom';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      refetchOnWindowFocus: false,
      retry: false,
    },
  },
});

function ScreenPending({ name }: { name: string }) {
  return (
    <div>
      <h1>{name}</h1>
      <p>screen pending</p>
    </div>
  );
}

const router = createBrowserRouter([
  {
    path: '/',
    element: <Navigate to="/overview" replace />,
  },
  {
    path: '/overview',
    element: <ScreenPending name="Overview" />,
  },
  {
    path: '/goals',
    element: <ScreenPending name="Goals" />,
  },
  {
    path: '/goals/:id',
    element: <ScreenPending name="Goal Details" />,
  },
  {
    path: '/packages',
    element: <ScreenPending name="Packages" />,
  },
  {
    path: '/settings',
    element: <ScreenPending name="Settings" />,
  },
  {
    path: '/runs',
    element: <Navigate to="/goals?view=activity" replace />,
  },
]);

export function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>
  );
}
