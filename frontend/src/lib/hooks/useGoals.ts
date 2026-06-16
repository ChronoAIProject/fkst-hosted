import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import {
  listGoals,
  getGoal,
  createGoal,
  updateGoal,
  deleteGoal,
  triggerGoal,
  CreateGoalRequest,
  UpdateGoalRequest,
  TriggerRequest,
} from '../api/goals';

/**
 * Hook to retrieve the list of goals.
 *
 * Choices and Rationales:
 * - staleTime (30000ms): Goals change relatively infrequently. Cache list for 30s to reduce unnecessary API requests.
 * - retry (false): Fail fast. If the goals list fails to fetch, we want the UI to display the error state immediately.
 */
export function useGoalsList(params?: { status?: string; limit?: number; offset?: number }) {
  return useQuery({
    queryKey: ['goals', params],
    queryFn: () => listGoals(params),
    staleTime: 30000,
    retry: false,
  });
}

/**
 * Hook to retrieve details for a specific goal.
 *
 * Choices and Rationales:
 * - staleTime (30000ms): Cached goal detail changes infrequently unless updated or triggered.
 * - retry (false): Fail fast. Do not hold up UI rendering with retries if a goal does not exist (404).
 * - enabled: Only executes the query if the goal ID parameter is provided.
 */
export function useGoal(id: string | undefined) {
  return useQuery({
    queryKey: ['goals', id],
    queryFn: () => getGoal(id!),
    staleTime: 30000,
    retry: false,
    enabled: !!id,
  });
}

/**
 * Hook to create a new goal.
 * Invalidates the goals list query on successful creation (201).
 */
export function useCreateGoal() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (req: CreateGoalRequest) => createGoal(req),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['goals'] });
    },
  });
}

/**
 * Hook to update an existing goal.
 * Invalidates the goals query to trigger refetches.
 */
export function useUpdateGoal() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ id, req }: { id: string; req: UpdateGoalRequest }) => updateGoal(id, req),
    onSuccess: (_, variables) => {
      queryClient.invalidateQueries({ queryKey: ['goals'] });
      queryClient.invalidateQueries({ queryKey: ['goals', variables.id] });
    },
  });
}

/**
 * Hook to delete a goal.
 * Invalidates the goals query on success.
 */
export function useDeleteGoal() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (id: string) => deleteGoal(id),
    onSuccess: (_, id) => {
      queryClient.invalidateQueries({ queryKey: ['goals'] });
      queryClient.invalidateQueries({ queryKey: ['goals', id] });
    },
  });
}

/**
 * Hook to trigger a goal, spawning a session.
 * Invalidates goals and sessions on success.
 */
export function useTriggerGoal() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ id, req }: { id: string; req: TriggerRequest }) => triggerGoal(id, req),
    onSuccess: (_, variables) => {
      queryClient.invalidateQueries({ queryKey: ['goals'] });
      queryClient.invalidateQueries({ queryKey: ['goals', variables.id] });
      queryClient.invalidateQueries({ queryKey: ['sessions'] });
    },
  });
}
