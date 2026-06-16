import { request, requestVoid } from './client';
import { PackageResponse, PackageFile } from './types';

export interface PackageDraft {
  name: string;
  files: PackageFile[];
  composed_deps: string[];
}

export interface ValidationReport {
  ok: boolean;
  errors: string[];
}

export interface ConformanceReport {
  status: 'ok' | 'failed' | 'skipped';
  errors: string[];
  skipped_reason: string | null;
}

export interface GenerateReport {
  package: PackageDraft;
  validation: ValidationReport;
  conformance: ConformanceReport;
  saved: boolean;
  save_error: string | null;
  attempts: number;
}

export interface ShareView {
  id: string;
  package_name: string;
  grantee_kind: 'user' | 'org';
  grantee_id: string;
  level: 'read' | 'use';
  granted_by: string;
  created_at: string;
}

/**
 * PUT /api/v1/packages/:name
 */
export async function updatePackage(
  name: string,
  pkg: { files: PackageFile[]; composed_deps?: string[] }
): Promise<PackageResponse> {
  const encodedName = encodeURIComponent(name);
  return request<PackageResponse>(`/api/v1/packages/${encodedName}`, {
    method: 'PUT',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify(pkg),
  });
}

/**
 * DELETE /api/v1/packages/:name
 */
export async function deletePackage(name: string): Promise<void> {
  const encodedName = encodeURIComponent(name);
  return requestVoid(`/api/v1/packages/${encodedName}`, {
    method: 'DELETE',
  });
}

/**
 * POST /api/v1/packages/:name/archive
 */
export async function archiveCreate(
  name: string,
  zipBytes: ArrayBuffer | Uint8Array
): Promise<{ name: string }> {
  const encodedName = encodeURIComponent(name);
  return request<{ name: string }>(`/api/v1/packages/${encodedName}/archive`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/zip',
    },
    body: zipBytes as BodyInit,
  });
}

/**
 * PUT /api/v1/packages/:name/archive
 */
export async function archiveReplace(
  name: string,
  zipBytes: ArrayBuffer | Uint8Array
): Promise<PackageResponse> {
  const encodedName = encodeURIComponent(name);
  return request<PackageResponse>(`/api/v1/packages/${encodedName}/archive`, {
    method: 'PUT',
    headers: {
      'Content-Type': 'application/zip',
    },
    body: zipBytes as BodyInit,
  });
}

/**
 * POST /api/v1/packages/generate
 */
export async function generatePackage(req: {
  description: string;
  name?: string;
  save?: boolean;
}): Promise<GenerateReport> {
  return request<GenerateReport>('/api/v1/packages/generate', {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify(req),
  });
}

/**
 * GET /api/v1/packages/:name/shares
 */
export async function listShares(name: string): Promise<ShareView[]> {
  const encodedName = encodeURIComponent(name);
  return request<ShareView[]>(`/api/v1/packages/${encodedName}/shares`);
}

/**
 * POST /api/v1/packages/:name/shares
 */
export async function createShare(
  name: string,
  share: {
    grantee_kind: 'user' | 'org';
    grantee_id: string;
    level: 'read' | 'use';
  }
): Promise<ShareView> {
  const encodedName = encodeURIComponent(name);
  return request<ShareView>(`/api/v1/packages/${encodedName}/shares`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify(share),
  });
}

/**
 * DELETE /api/v1/packages/:name/shares/:shareId
 */
export async function deleteShare(name: string, shareId: string): Promise<void> {
  const encodedName = encodeURIComponent(name);
  const encodedShareId = encodeURIComponent(shareId);
  return requestVoid(`/api/v1/packages/${encodedName}/shares/${encodedShareId}`, {
    method: 'DELETE',
  });
}
