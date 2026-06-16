import { createBrowserRouter, Navigate, RouterProvider, useParams, useSearchParams, useOutletContext } from 'react-router-dom';
import { QueryClient } from '@tanstack/react-query';
import { PersistQueryClientProvider } from '@tanstack/react-query-persist-client';
import { queryPersister, PERSIST_MAX_AGE, PERSIST_BUSTER } from '../lib/persist/persister';
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
      // gcTime must be >= the persist maxAge, else restored entries are
      // garbage-collected before the UI can read them (ARCHITECTURE.md §8).
      gcTime: PERSIST_MAX_AGE,
    },
  },
});

import { useGoalsList, useGoal } from '../lib/hooks/useGoals';
import IssuesScreen from '../screens/issues/issues-screen';

function OverviewRoute() {
  const context = useOutletContext<ShellOutletContext>();
  const { data: goals, isLoading, isError } = useGoalsList();

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

  // On error (e.g. backend unreachable), render the Overview with no data so it
  // shows its honest empty/unknown state — never replace the whole screen with a
  // raw error (ARCHITECTURE.md: unreachable → honest gap, not a dead end).
  return <Overview goals={isError ? undefined : goals} onNewGoal={context?.onNewGoal} />;
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
    <PersistQueryClientProvider
      client={queryClient}
      persistOptions={{
        persister: queryPersister,
        maxAge: PERSIST_MAX_AGE,
        buster: PERSIST_BUSTER,
        dehydrateOptions: {
          // Only successful GET reads are persisted — never mutations,
          // errors, or `unknown` placeholders (ARCHITECTURE.md §8).
          shouldDehydrateQuery: (query) => query.state.status === 'success',
          shouldDehydrateMutation: () => false,
        },
      }}
    >
      <SessionRegistryProvider>
        <RouterProvider router={router} future={{ v7_startTransition: true }} />
        <Toaster />
      </SessionRegistryProvider>
    </PersistQueryClientProvider>
  );
}
