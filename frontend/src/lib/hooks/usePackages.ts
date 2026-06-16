import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { getPackagesList, getPackage, createPackage } from '../api/client';
import { NewPackage } from '../api/types';

/**
 * Hook to retrieve the list of all package names.
 *
 * Choices and Rationales:
 * - staleTime (30000ms): Packages are created relatively infrequently. Cache list for 30s to reduce unnecessary API requests.
 * - retry (false): Fail fast. If the package list fails to fetch, we want the UI to display the error state immediately.
 */
export function usePackagesList() {
  return useQuery({
    queryKey: ['packages'],
    queryFn: getPackagesList,
    staleTime: 30000,
    retry: false,
  });
}

/**
 * Hook to retrieve details for a specific package.
 *
 * Choices and Rationales:
 * - staleTime (30000ms): The UI only creates packages in v1, so cached package detail changes infrequently.
 * - retry (false): Fail fast. Do not hold up UI rendering with retries if a package name is invalid or does not exist (404).
 * - enabled: Only executes the query if the package name parameter is provided.
 */
export function usePackage(name: string | undefined) {
  return useQuery({
    queryKey: ['packages', name],
    queryFn: () => getPackage(name!),
    staleTime: 30000,
    retry: false,
    enabled: !!name,
  });
}

/**
 * Hook to create a new package.
 * Invalidates the packages list query on successful creation (201).
 * If a conflict (409) occurs, the error surfaces as an ApiError.
 */
export function useCreatePackage() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (pkg: NewPackage) => createPackage(pkg),
    onSuccess: () => {
      // Invalidate the package list query to trigger a refetch of new packages
      queryClient.invalidateQueries({ queryKey: ['packages'] });
    },
  });
}
