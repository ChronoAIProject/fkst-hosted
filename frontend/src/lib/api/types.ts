export interface PackageFile {
  path: string;
  content: string;
}

export interface PackageResponse {
  name: string;
  files: PackageFile[];
  composed_deps: string[];
  created_at: string;
  updated_at: string;
}

/**
 * PackageSummary represents the short name identifier of a package.
 * GET /api/v1/packages returns string[].
 */
export type PackageSummary = string;

export interface NewPackage {
  name: string;
  files: PackageFile[];
  composed_deps?: string[];
}

export type SessionStatus =
  | 'pending'
  | 'validating'
  | 'running'
  | 'stopping'
  | 'stopped'
  | 'failed';

export interface SessionView {
  id: string;
  package_name: string;
  status: SessionStatus;
  pod_id: string | null;
  fencing_token: number | null;
  pid: number | null;
  runtime_dir: string | null;
  error: string | null;
  created_at: string;
  started_at: string | null;
  stopped_at: string | null;
}

export interface CreateSessionResponse {
  id: string;
  status: SessionStatus;
}

export interface StopResponse {
  status: SessionStatus;
}

export interface HealthResponse {
  status: 'ok' | 'degraded';
  mongo: 'up' | 'down';
  version: string;
}

export interface ApiErrorBody {
  error: string;
  message: string;
}

/**
 * Type guard for ApiErrorBody.
 */
export function isApiErrorBody(body: unknown): body is ApiErrorBody {
  if (typeof body !== 'object' || body === null) {
    return false;
  }
  const candidate = body as Record<string, unknown>;
  return (
    'error' in candidate &&
    typeof candidate.error === 'string' &&
    'message' in candidate &&
    typeof candidate.message === 'string'
  );
}

/**
 * Type guard for HealthResponse.
 */
export function isHealthResponse(body: unknown): body is HealthResponse {
  if (typeof body !== 'object' || body === null) {
    return false;
  }
  const candidate = body as Record<string, unknown>;
  return (
    'status' in candidate &&
    (candidate.status === 'ok' || candidate.status === 'degraded') &&
    'mongo' in candidate &&
    (candidate.mongo === 'up' || candidate.mongo === 'down') &&
    'version' in candidate &&
    typeof candidate.version === 'string'
  );
}

export interface AccountView {
  connection_id: string;
  login: string;
  primary: boolean;
}
