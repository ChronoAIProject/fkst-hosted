import { apiBase } from '../env';
import {
  HealthResponse,
  PackageSummary,
  PackageResponse,
  NewPackage,
  CreateSessionResponse,
  SessionView,
  StopResponse,
  ApiErrorBody,
  isApiErrorBody,
  isHealthResponse,
} from './types';
import { getAccessToken, handleUnauthorized } from '../auth/token';

/**
 * Custom error class representing an API failure.
 * Carries the HTTP status code (or 0 for network/fetch failures) and the parsed error body.
 */
export class ApiError extends Error {
  status: number;
  body: ApiErrorBody | HealthResponse | null;

  constructor(status: number, body: ApiErrorBody | HealthResponse | null, message: string) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
    this.body = body;
    // Restore prototype chain for ES5/TS environments
    Object.setPrototypeOf(this, ApiError.prototype);
  }
}

/**
 * Helper to build RequestInit options including the Authorization header if a token is present.
 */
function buildOptions(options?: RequestInit): RequestInit {
  const token = getAccessToken();
  if (!token) {
    return options || {};
  }
  const headers = new Headers(options?.headers);
  headers.set('Authorization', `Bearer ${token}`);
  return {
    ...options,
    headers,
  };
}

/**
 * Helper to perform typed requests expecting a JSON response body.
 * Throws ApiError if response is not ok or is not JSON.
 */
async function request<T>(path: string, options?: RequestInit): Promise<T> {
  const url = `${apiBase()}${path}`;
  let response: Response;

  try {
    response = await fetch(url, buildOptions(options));
  } catch (err) {
    const message = err instanceof Error ? err.message : 'Network failure';
    throw new ApiError(0, null, message);
  }

  if (!response.ok) {
    if (response.status === 401) {
      handleUnauthorized();
    }
    let body: unknown = null;
    try {
      body = await response.json();
    } catch {
      // Ignore body parsing failures
    }

    let parsedBody: ApiErrorBody | HealthResponse | null = null;
    if (isApiErrorBody(body)) {
      parsedBody = body;
    } else if (isHealthResponse(body)) {
      parsedBody = body;
    }

    throw new ApiError(
      response.status,
      parsedBody,
      isApiErrorBody(body) ? body.message : `Request failed with status ${response.status}`
    );
  }

  const contentType = response.headers.get('content-type');
  if (contentType && contentType.includes('application/json')) {
    try {
      return (await response.json()) as T;
    } catch {
      throw new ApiError(
        response.status,
        null,
        'Failed to parse JSON response content'
      );
    }
  }

  throw new ApiError(
    response.status,
    null,
    'Expected JSON response body but received none'
  );
}

/**
 * Helper to perform requests expecting no response body (void).
 * Throws ApiError if response is not ok.
 */
export async function requestVoid(path: string, options?: RequestInit): Promise<void> {
  const url = `${apiBase()}${path}`;
  let response: Response;

  try {
    response = await fetch(url, buildOptions(options));
  } catch (err) {
    const message = err instanceof Error ? err.message : 'Network failure';
    throw new ApiError(0, null, message);
  }

  if (!response.ok) {
    if (response.status === 401) {
      handleUnauthorized();
    }
    let body: unknown = null;
    try {
      body = await response.json();
    } catch {
      // Ignore body parsing failures
    }

    let parsedBody: ApiErrorBody | HealthResponse | null = null;
    if (isApiErrorBody(body)) {
      parsedBody = body;
    } else if (isHealthResponse(body)) {
      parsedBody = body;
    }

    throw new ApiError(
      response.status,
      parsedBody,
      isApiErrorBody(body) ? body.message : `Request failed with status ${response.status}`
    );
  }
}

/**
 * GET /api/v1/health
 */
export async function getHealth(): Promise<HealthResponse> {
  return request<HealthResponse>('/api/v1/health');
}

/**
 * GET /api/v1/packages
 */
export async function getPackagesList(): Promise<PackageSummary[]> {
  return request<PackageSummary[]>('/api/v1/packages');
}

/**
 * GET /api/v1/packages/:name
 */
export async function getPackage(name: string): Promise<PackageResponse> {
  // Axum path segment is percent-decoded, but we should encode it for safety in URL
  const encodedName = encodeURIComponent(name);
  return request<PackageResponse>(`/api/v1/packages/${encodedName}`);
}

/**
 * POST /api/v1/packages
 */
export async function createPackage(pkg: NewPackage): Promise<{ name: string }> {
  return request<{ name: string }>('/api/v1/packages', {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify(pkg),
  });
}

/**
 * POST /api/v1/sessions
 */
export async function createSession(packageName: string): Promise<CreateSessionResponse> {
  return request<CreateSessionResponse>('/api/v1/sessions', {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({ package_name: packageName }),
  });
}

/**
 * GET /api/v1/sessions/:id
 */
export async function getSession(id: string): Promise<SessionView> {
  const encodedId = encodeURIComponent(id);
  return request<SessionView>(`/api/v1/sessions/${encodedId}`);
}

/**
 * POST /api/v1/sessions/:id/stop
 */
export async function stopSession(id: string): Promise<StopResponse> {
  const encodedId = encodeURIComponent(id);
  return request<StopResponse>(`/api/v1/sessions/${encodedId}/stop`, {
    method: 'POST',
  });
}
