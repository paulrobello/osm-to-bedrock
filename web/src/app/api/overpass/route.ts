// web/src/app/api/overpass/route.ts
import { buildQuery, type FeatureFilter } from '@/lib/overpass';

const DEFAULT_OVERPASS_URL = 'https://overpass-api.de/api/interpreter';
const FETCH_TIMEOUT_MS = 35_000;

/**
 * Approved Overpass API hostnames. Only HTTPS URLs whose host appears here
 * are accepted — all others are rejected to prevent SSRF.
 */
const ALLOWED_OVERPASS_HOSTS = new Set([
  'overpass-api.de',
  'overpass.kumi.systems',
  'overpass.openstreetmap.ru',
  'maps.mail.ru',
  'overpass.osm.ch',
]);

/**
 * Returns true if `rawUrl` is a safe Overpass endpoint (HTTPS + approved host).
 */
function isAllowedOverpassUrl(rawUrl: string): boolean {
  let parsed: URL;
  try {
    parsed = new URL(rawUrl);
  } catch {
    return false;
  }
  if (parsed.protocol !== 'https:') return false;
  return ALLOWED_OVERPASS_HOSTS.has(parsed.hostname);
}

export async function POST(request: Request): Promise<Response> {
  let body: { bbox?: unknown; filter?: unknown; overpass_url?: unknown };
  try {
    body = (await request.json()) as { bbox?: unknown; filter?: unknown; overpass_url?: unknown };
  } catch {
    return Response.json({ error: 'Invalid JSON body' }, { status: 400 });
  }

  const { bbox } = body;
  if (
    !Array.isArray(bbox) ||
    bbox.length !== 4 ||
    bbox.some((v) => typeof v !== 'number')
  ) {
    return Response.json(
      { error: 'bbox must be an array of 4 numbers [south, west, north, east]' },
      { status: 400 }
    );
  }

  const requestedUrl =
    typeof body.overpass_url === 'string' ? body.overpass_url.trim() : '';

  // Reject non-empty overpass_url values that fail the allowlist check.
  // An empty/absent value silently falls back to the default endpoint.
  if (requestedUrl && !isAllowedOverpassUrl(requestedUrl)) {
    return Response.json(
      {
        error:
          'overpass_url must be an HTTPS URL pointing to an approved Overpass host',
      },
      { status: 400 }
    );
  }

  const overpassUrl = requestedUrl || DEFAULT_OVERPASS_URL;

  const filter = body.filter as FeatureFilter | undefined;
  const query = buildQuery(bbox as [number, number, number, number], filter);

  const controller = new AbortController();
  const timerId = setTimeout(() => controller.abort(), FETCH_TIMEOUT_MS);

  try {
    const res = await fetch(overpassUrl, {
      method: 'POST',
      headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
      body: `data=${encodeURIComponent(query)}`,
      signal: controller.signal,
    });

    if (!res.ok) {
      const text = await res.text().catch(() => res.statusText);
      return Response.json(
        { error: `Overpass API error (${res.status}): ${text}` },
        { status: 502 }
      );
    }

    const data: unknown = await res.json();
    return Response.json(data);
  } catch (err: unknown) {
    const message =
      err instanceof Error
        ? err.name === 'AbortError'
          ? 'Overpass request timed out'
          : err.message
        : 'Unknown error';
    return Response.json({ error: message }, { status: 502 });
  } finally {
    clearTimeout(timerId);
  }
}
