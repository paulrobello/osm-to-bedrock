const NOMINATIM_URL = 'https://nominatim.openstreetmap.org/search';
const FETCH_TIMEOUT_MS = 10_000;

export async function GET(request: Request): Promise<Response> {
  const { searchParams } = new URL(request.url);
  const q = searchParams.get('q');

  // SEC-012: enforce a maximum length on the query string to prevent abuse.
  const MAX_Q_LENGTH = 500;
  if (!q || q.trim().length === 0) {
    return Response.json({ error: 'Missing query parameter "q"' }, { status: 400 });
  }
  if (q.length > MAX_Q_LENGTH) {
    return Response.json(
      { error: `Query parameter "q" must not exceed ${MAX_Q_LENGTH} characters` },
      { status: 400 }
    );
  }

  const url = new URL(NOMINATIM_URL);
  url.searchParams.set('q', q.trim());
  url.searchParams.set('format', 'json');
  url.searchParams.set('limit', '5');

  const controller = new AbortController();
  const timerId = setTimeout(() => controller.abort(), FETCH_TIMEOUT_MS);

  try {
    const res = await fetch(url.toString(), {
      headers: {
        'User-Agent': 'osm-to-bedrock/1.0 (https://github.com/paulrobello/osm-to-bedrock)',
        'Accept': 'application/json',
      },
      signal: controller.signal,
    });

    if (!res.ok) {
      const text = await res.text().catch(() => res.statusText);
      return Response.json(
        { error: `Nominatim error (${res.status}): ${text}` },
        { status: 502 }
      );
    }

    const data: unknown = await res.json();
    return Response.json(data);
  } catch (err: unknown) {
    const message =
      err instanceof Error
        ? err.name === 'AbortError'
          ? 'Geocoding request timed out'
          : err.message
        : 'Unknown error';
    return Response.json({ error: message }, { status: 502 });
  } finally {
    clearTimeout(timerId);
  }
}
