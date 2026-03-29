/**
 * Centralised Rust API configuration.
 *
 * All proxy routes must import from this module instead of defining their own
 * `RUST_API_URL` constant and timeout values.  This is the single source of
 * truth for the backend URL and per-route timeout budgets.
 */

/** Base URL of the Rust API server. */
export const RUST_API_URL =
  process.env.NEXT_PUBLIC_API_URL || 'http://localhost:3002';

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
