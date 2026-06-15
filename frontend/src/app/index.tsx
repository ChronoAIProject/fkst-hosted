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
import { AuthCallback } from './auth-callback';

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      refetchOnWindowFocus: false,
      retry: false,
    },
  },
});

import { useGoalsList, useGoal } from '../lib/hooks/useGoals';
import IssuesScreen from '../screens/issues/issues-screen';

function OverviewRoute() {
  const context = useOutletContext<ShellOutletContext>();
  const { data: goals, isLoading, isError, error } = useGoalsList();

  useEffect(() => {
    document.title = 'FKST — Overview';
  }, []);

  if (isLoading) {
    return (
      <div className="flex flex-col items-center justify-center min-h-[300px] text-ghost font-mono text-[12px]">
        loading overview...
      </div>
    );
  }

  if (isError) {
    return (
      <div className="flex flex-col items-center justify-center min-h-[300px] text-red font-mono text-[12px]">
        failed to load goals: {String(error)}
      </div>
    );
  }

  return <Overview goals={goals} onNewGoal={context?.onNewGoal} />;
}

function GoalsRoute() {
  const [searchParams, setSearchParams] = useSearchParams();
  const context = useOutletContext<ShellOutletContext>();
  const view = searchParams.get('view') === 'activity' ? 'activity' : 'issues';
  const { data: goals, isLoading, isError, error } = useGoalsList();
  
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

  return (
    <Goals 
      view={view} 
      goals={goals}
      isLoadingGoals={isLoading}
      isErrorGoals={isError}
      goalsError={error}
      onNewGoal={context?.onNewGoal} 
      onViewChange={handleViewChange} 
    />
  );
}

function GoalRoute() {
  const { id } = useParams<{ id: string }>();
  const { data: goal, isLoading, isError, error } = useGoal(id);

  useEffect(() => {
    document.title = id ? `FKST — Goal #${id}` : 'FKST — Goal Details';
  }, [id]);

  if (isLoading) {
    return (
      <div className="flex flex-col items-center justify-center min-h-[300px] text-ghost font-mono text-[12px]">
        loading goal details...
      </div>
    );
  }

  if (isError) {
    return (
      <div className="flex flex-col items-center justify-center min-h-[300px] text-red font-mono text-[12px]">
        failed to load goal #{id}: {String(error)}
      </div>
    );
  }

  if (!goal) {
    return (
      <div className="flex flex-col items-center justify-center min-h-[300px] text-ghost font-mono text-[12px]">
        goal not found
      </div>
    );
  }

  return <Goal goal={goal} />;
}

function IssuesRoute() {
  useEffect(() => {
    document.title = 'FKST — Issues';
  }, []);
  return <IssuesScreen />;
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
      path: '/auth/callback',
      element: <AuthCallback />,
    },
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
          path: 'issues',
          element: <IssuesRoute />,
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
