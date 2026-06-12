import { createBrowserRouter, Navigate, RouterProvider, useParams, useSearchParams, useOutletContext } from 'react-router-dom';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { Shell, ShellOutletContext } from './shell';
import { SessionRegistryProvider } from '../lib/hooks/session-registry';
import { Toaster } from '../components/primitives/toaster';
import { Overview } from '../screens/overview/overview';
import { Goals } from '../screens/goals/goals';
import { Goal } from '../screens/goal/goal';
import PackagesScreen from '../screens/packages/packages-screen';
import SettingsScreen from '../screens/settings/settings-screen';
import { useEffect, useState } from 'react';

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      refetchOnWindowFocus: false,
      retry: false,
    },
  },
});

function OverviewRoute() {
  const context = useOutletContext<ShellOutletContext>();
  useEffect(() => {
    document.title = 'FKST — Overview';
  }, []);
  return <Overview onNewGoal={context?.onNewGoal} />;
}

function GoalsRoute() {
  const [searchParams, setSearchParams] = useSearchParams();
  const context = useOutletContext<ShellOutletContext>();
  const view = searchParams.get('view') === 'activity' ? 'activity' : 'issues';
  
  useEffect(() => {
    document.title = 'FKST — Goals';
  }, []);

  const handleViewChange = (newView: 'issues' | 'activity') => {
    if (newView === 'issues') {
      const nextParams = new URLSearchParams(searchParams);
      nextParams.delete('view');
      setSearchParams(nextParams);
    } else {
      setSearchParams({ view: newView });
    }
  };

  return <Goals view={view} onNewGoal={context?.onNewGoal} onViewChange={handleViewChange} />;
}

function GoalRoute() {
  const { id } = useParams<{ id: string }>();
  useEffect(() => {
    document.title = id ? `FKST — Goal #${id}` : 'FKST — Goal Details';
  }, [id]);
  return <Goal goalId={id} />;
}

function PackagesRoute() {
  useEffect(() => {
    document.title = 'FKST — Packages';
  }, []);
  return <PackagesScreen />;
}

function SettingsRoute() {
  useEffect(() => {
    document.title = 'FKST — Settings';
  }, []);
  return <SettingsScreen />;
}

export function App() {
  const [router] = useState(() => createBrowserRouter([
    {
      path: '/',
      element: <Shell />,
      children: [
        {
          path: '',
          element: <Navigate to="/overview" replace />,
        },
        {
          path: 'overview',
          element: <OverviewRoute />,
        },
        {
          path: 'goals',
          element: <GoalsRoute />,
        },
        {
          path: 'goals/:id',
          element: <GoalRoute />,
        },
        {
          path: 'packages',
          element: <PackagesRoute />,
        },
        {
          path: 'settings',
          element: <SettingsRoute />,
        },
        {
          path: 'runs',
          element: <Navigate to="/goals?view=activity" replace />,
        },
      ],
    },
  ], {
    future: {
      v7_relativeSplatPath: true,
    },
  }));

  return (
    <QueryClientProvider client={queryClient}>
      <SessionRegistryProvider>
        <RouterProvider router={router} future={{ v7_startTransition: true }} />
        <Toaster />
      </SessionRegistryProvider>
    </QueryClientProvider>
  );
}
