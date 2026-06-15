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
import { useGoalsList, useGoal } from '../lib/hooks/useGoals';
import IssuesScreen from '../screens/issues/issues-screen';
import { goalStatusPresentation } from '../lib/api/goal-status';

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
  const { data: goals, isLoading, isError, error } = useGoalsList();

  useEffect(() => {
    document.title = 'FKST — Overview';
  }, []);

  if (isLoading) {
    return <div className="p-6 text-dim font-mono text-[13.5px]">Loading overview...</div>;
  }

  if (isError) {
    return (
      <div className="p-6 text-red font-mono text-[13.5px]">
        Error loading overview: {error instanceof Error ? error.message : 'Unknown error'}
      </div>
    );
  }

  return <Overview goals={goals} statusPresentation={goalStatusPresentation} onNewGoal={context?.onNewGoal} />;
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

  if (isLoading) {
    return <div className="p-6 text-dim font-mono text-[13.5px]">Loading goals...</div>;
  }

  if (isError) {
    return (
      <div className="p-6 text-red font-mono text-[13.5px]">
        Error loading goals: {error instanceof Error ? error.message : 'Unknown error'}
      </div>
    );
  }

  return (
    <Goals
      view={view}
      goals={goals}
      statusPresentation={goalStatusPresentation}
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
    return <div className="p-6 text-dim font-mono text-[13.5px]">Loading goal details...</div>;
  }

  if (isError) {
    return (
      <div className="p-6 text-red font-mono text-[13.5px]">
        Error loading goal details: {error instanceof Error ? error.message : 'Unknown error'}
      </div>
    );
  }

  return (
    <Goal
      goalId={goal?.id}
      title={goal?.title}
      state={goal?.status}
      isReal={true}
    />
  );
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
          element: <IssuesScreen />,
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
