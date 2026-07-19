// Fetch layer for a session's outcome files (spec B2/B3). Each function takes
// the caller's `apiFetch` (the token-bearing fetch from useAuth) so components
// stay testable with a plain stub.
//
// Media auth is single-path by DESIGN (spec F1): an `<img>/<video>` tag cannot
// carry the Bearer header, so instead of a naked blob URL we fetch the bytes
// through `apiFetch` and hand the component a `Blob`. The component turns that
// into an object URL for preview (and revokes it on unmount); the Download
// button re-fetches with `download=1` and saves the same way. One auth path,
// no token ever in a URL.

import { assertShape, type ApiFetch } from './canvas';
import type { SessionOutcomes } from './types';

/** The backend caps a single blob at 25 MiB; past that it returns 413 and the
 *  UI points the user at GitHub instead of previewing. Kept in sync with the
 *  backend `max_bytes` so the "too large" copy matches the real limit. */
export const MAX_BLOB_BYTES = 25 * 1024 * 1024;

/** Outcome of a blob fetch: the bytes, or a typed failure. `tooLarge` (413) is
 *  distinguished so the component can show the "open on GitHub" affordance. */
export type BlobResult =
  | { ok: true; blob: Blob }
  | { ok: false; tooLarge: boolean; status: number };

/** GET /api/v1/repos/{owner}/{name}/sessions/{issue}/outcomes — the session's
 *  devloop PRs, each with its committed files (grouped by PR on the backend). */
export async function getSessionOutcomes(
  apiFetch: ApiFetch,
  owner: string,
  name: string,
  issue: number
): Promise<SessionOutcomes> {
  const res = await apiFetch(
    `/api/v1/repos/${encodeURIComponent(owner)}/${encodeURIComponent(name)}/sessions/${issue}/outcomes`
  );
  if (!res.ok) throw new Error(`session outcomes failed: ${res.status}`);
  const body = (await res.json()) as SessionOutcomes;
  assertShape(Array.isArray(body?.prs), 'session outcomes');
  return body;
}

/** GET /api/v1/repos/{owner}/{name}/blob/{sha} — one committed file's raw bytes.
 *  `filename` drives the server's `Content-Type` guess (and the download name);
 *  `download` flips `Content-Disposition` to attachment. Returns a `Blob` on
 *  success, or a typed failure (413 ⇒ `tooLarge`) — never throws on an HTTP
 *  error, so callers branch on the result instead of a try/catch. */
export async function fetchBlob(
  apiFetch: ApiFetch,
  owner: string,
  name: string,
  sha: string,
  filename: string,
  download = false
): Promise<BlobResult> {
  const query = new URLSearchParams({ name: filename });
  if (download) query.set('download', '1');
  const res = await apiFetch(
    `/api/v1/repos/${encodeURIComponent(owner)}/${encodeURIComponent(name)}/blob/${encodeURIComponent(sha)}?${query.toString()}`
  );
  if (!res.ok) return { ok: false, tooLarge: res.status === 413, status: res.status };
  return { ok: true, blob: await res.blob() };
}

/** Trigger a browser "Save as" for an already-fetched blob. Wraps the object
 *  URL in a synthetic anchor click and revokes the URL after the click so no
 *  handle leaks. Isolated here (DOM side effect) to keep the fetch fns pure. */
export function saveBlob(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = filename;
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  // Revoke after the click has been dispatched; a microtask is enough.
  setTimeout(() => URL.revokeObjectURL(url), 0);
}
