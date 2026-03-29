import { RUST_API_URL, TIMEOUTS } from '@/lib/api-config';
const FETCH_TIMEOUT_MS = TIMEOUTS.SHORT;

export async function GET(
  _request: Request,
  { params }: { params: Promise<{ id: string }> }
): Promise<Response> {
  const { id } = await params;

  if (!id) {
    return Response.json({ error: 'Missing job id' }, { status: 400 });
  }

  const controller = new AbortController();
  const timerId = setTimeout(() => controller.abort(), FETCH_TIMEOUT_MS);

  try {
    const res = await fetch(
      `${RUST_API_URL}/status/${encodeURIComponent(id)}`,
      {
        method: 'GET',
        signal: controller.signal,
      }
    );

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
          ? 'Status request timed out'
          : err.message
        : 'Unknown error';
    return Response.json({ error: message }, { status: 502 });
  } finally {
    clearTimeout(timerId);
  }
}
