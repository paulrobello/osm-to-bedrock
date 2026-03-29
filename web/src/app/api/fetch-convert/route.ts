// web/src/app/api/fetch-convert/route.ts
import { RUST_API_URL, TIMEOUTS } from '@/lib/api-config';
const FETCH_TIMEOUT_MS = TIMEOUTS.FETCH_CONVERT;

export async function POST(request: Request): Promise<Response> {
  let body: unknown;
  try {
    body = await request.json();
  } catch {
    return Response.json({ error: 'Invalid JSON body' }, { status: 400 });
  }

  const controller = new AbortController();
  const timerId = setTimeout(() => controller.abort(), FETCH_TIMEOUT_MS);

  try {
    const res = await fetch(`${RUST_API_URL}/fetch-convert`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
      signal: controller.signal,
    });

    if (!res.ok) {
      const text = await res.text().catch(() => res.statusText);
      return Response.json(
        { error: `Rust API error (${res.status}): ${text}` },
        { status: 502 }
      );
    }

    const data: unknown = await res.json();
    return Response.json(data);
  } catch (err: unknown) {
    const message =
      err instanceof Error
        ? err.name === 'AbortError'
          ? 'Fetch-convert request timed out'
          : err.message
        : 'Unknown error';
    return Response.json({ error: message }, { status: 502 });
  } finally {
    clearTimeout(timerId);
  }
}
