/**
 * Centralised Rust API configuration.
 *
 * All proxy routes must import from this module instead of defining their own
 * base-URL constant and timeout values. This is the single source of truth for
 * the backend URL, per-route timeout budgets, and the standard proxy helper.
 *
 * The base URL is read from the server-side `RUST_API_URL` env var. It is
 * intentionally NOT `NEXT_PUBLIC_`-prefixed, so the value is never inlined
 * into client bundles at build time and can be overridden at runtime (e.g.
 * `docker run -e RUST_API_URL=...`) without rebuilding the frontend. Browser
 * code never calls the Rust API directly — it always goes through the Next.js
 * proxy routes under `app/api/` (one `route.ts` per subdirectory), which run
 * server-side.
 */

/** Base URL of the Rust API server. Server-side only (never reaches the browser). */
export const RUST_API_URL = process.env.RUST_API_URL || 'http://localhost:3002';

/** Timeout budgets per route (milliseconds). */
export const TIMEOUTS = {
  /** Short read-only queries (health, cache list, status polls). */
  SHORT: 10_000,
  /** File uploads and parse operations. */
  UPLOAD: 30_000,
  /** Full conversion jobs — PBF upload + world build. */
  CONVERT: 60_000,
  /** Fetch-convert (Overpass round-trip + world build). */
  FETCH_CONVERT: 120_000,
  /** Terrain-only generation — SRTM download may be slow. */
  TERRAIN_CONVERT: 300_000,
  /** Large file downloads. */
  DOWNLOAD: 120_000,
} as const;

/**
 * Forwards a request to the Rust API with a timeout and returns a `Response`
 * ready to return directly from a Next.js route handler.
 *
 * Standardised error envelope (all return HTTP 502):
 * - Upstream non-2xx → `{ error: "Rust API error (NNN): <upstream body>" }`
 * - Timeout (AbortError) → `{ error: "<timeoutLabel> request timed out" }`
 * - Other network/decode error → `{ error: "<err.message>" }`
 *
 * On success the upstream JSON body is passed through unchanged.
 *
 * Routes whose success path is not a plain JSON passthrough do not use this
 * helper: `download` streams the upstream body with custom headers, while
 * `cache` and `health` degrade silently on error (they never return 502).
 */
export async function proxyToRust(
  path: string,
  options: {
    method: 'GET' | 'POST';
    body?: BodyInit;
    headers?: Record<string, string>;
    timeoutMs: number;
    /** Label used in the timeout message, e.g. "Convert" → "Convert request timed out". */
    timeoutLabel: string;
  },
): Promise<Response> {
  const controller = new AbortController();
  const timerId = setTimeout(() => controller.abort(), options.timeoutMs);
  try {
    const res = await fetch(`${RUST_API_URL}${path}`, {
      method: options.method,
      body: options.body,
      headers: options.headers,
      signal: controller.signal,
    });
    if (!res.ok) {
      const text = await res.text().catch(() => res.statusText);
      return Response.json(
        { error: `Rust API error (${res.status}): ${text}` },
        { status: 502 },
      );
    }
    const data: unknown = await res.json();
    return Response.json(data);
  } catch (err: unknown) {
    const message =
      err instanceof Error
        ? err.name === 'AbortError'
          ? `${options.timeoutLabel} request timed out`
          : err.message
        : 'Unknown error';
    return Response.json({ error: message }, { status: 502 });
  } finally {
    clearTimeout(timerId);
  }
}
