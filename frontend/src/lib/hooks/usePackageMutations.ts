import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import {
  updatePackage,
  deletePackage,
  archiveCreate,
  archiveReplace,
  generatePackage,
  listShares,
  createShare,
  deleteShare,
} from '../api/packages-extra';
import { PackageFile } from '../api/types';

/**
 * Hook to update an existing package's files and dependencies.
 * Invalidates ['packages'] and the specific ['packages', name].
 */
export function useUpdatePackage() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      name,
      pkg,
    }: {
      name: string;
      pkg: { files: PackageFile[]; composed_deps?: string[] };
    }) => updatePackage(name, pkg),
    onSuccess: (_, variables) => {
      queryClient.invalidateQueries({ queryKey: ['packages'] });
      queryClient.invalidateQueries({ queryKey: ['packages', variables.name] });
    },
  });
}

/**
 * Hook to delete a package.
 * Invalidates ['packages'].
 */
export function useDeletePackage() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (name: string) => deletePackage(name),
    onSuccess: (_, name) => {
      queryClient.invalidateQueries({ queryKey: ['packages'] });
      queryClient.invalidateQueries({ queryKey: ['packages', name] });
    },
  });
}

/**
 * Hook to create a package from a zip archive.
 * Invalidates ['packages'].
 */
export function useArchiveCreate() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ name, zipBytes }: { name: string; zipBytes: ArrayBuffer | Uint8Array }) =>
      archiveCreate(name, zipBytes),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['packages'] });
    },
  });
}

/**
 * Hook to replace a package from a zip archive.
 * Invalidates ['packages'] and the specific ['packages', name].
 */
export function useArchiveReplace() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ name, zipBytes }: { name: string; zipBytes: ArrayBuffer | Uint8Array }) =>
      archiveReplace(name, zipBytes),
    onSuccess: (_, variables) => {
      queryClient.invalidateQueries({ queryKey: ['packages'] });
      queryClient.invalidateQueries({ queryKey: ['packages', variables.name] });
    },
  });
}

/**
 * Hook to generate a package via AI.
 * If saved was true in request, invalidates ['packages'].
 */
export function useGeneratePackage() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (req: { description: string; name?: string; save?: boolean }) =>
      generatePackage(req),
    onSuccess: (data, variables) => {
      if (variables.save && data.saved) {
        queryClient.invalidateQueries({ queryKey: ['packages'] });
      }
    },
  });
}

/**
 * Hook to list all share grants for a package.
 *
 * Choices and Rationales:
 * - staleTime (30000ms): Shares are modified infrequently. Cache for 30s.
 * - retry (false): Fail fast.
 * - enabled: Only executes if package name is provided.
 */
export function useShares(name: string | undefined) {
  return useQuery({
    queryKey: ['shares', name],
    queryFn: () => listShares(name!),
    staleTime: 30000,
    retry: false,
    enabled: !!name,
  });
}

/**
 * Hook to create a package share.
 * Invalidates ['shares', name].
 */
export function useCreateShare() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      name,
      share,
    }: {
      name: string;
      share: {
        grantee_kind: 'user' | 'org';
        grantee_id: string;
        level: 'read' | 'use';
      };
    }) => createShare(name, share),
    onSuccess: (_, variables) => {
      queryClient.invalidateQueries({ queryKey: ['shares', variables.name] });
    },
  });
}

/**
 * Hook to revoke/delete a package share.
 * Invalidates ['shares', name].
 */
export function useDeleteShare() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ name, shareId }: { name: string; shareId: string }) =>
      deleteShare(name, shareId),
    onSuccess: (_, variables) => {
      queryClient.invalidateQueries({ queryKey: ['shares', variables.name] });
    },
  });
}
