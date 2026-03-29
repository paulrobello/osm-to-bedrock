import { RUST_API_URL, TIMEOUTS } from '@/lib/api-config';
const FETCH_TIMEOUT_MS = TIMEOUTS.DOWNLOAD;

export async function GET(request: Request): Promise<Response> {
  const { searchParams } = new URL(request.url);
  const jobId = searchParams.get('id');

  if (!jobId) {
    return Response.json({ error: 'Missing id query parameter' }, { status: 400 });
  }

  const controller = new AbortController();
  const timerId = setTimeout(() => controller.abort(), FETCH_TIMEOUT_MS);

  try {
    const res = await fetch(
      `${RUST_API_URL}/download/${encodeURIComponent(jobId)}`,
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

    // Determine filename from Content-Disposition or fallback
    const contentDisposition = res.headers.get('content-disposition');
    const contentType =
      res.headers.get('content-type') ?? 'application/octet-stream';

    const headers = new Headers();
    headers.set('content-type', contentType);
    if (contentDisposition) {
      headers.set('content-disposition', contentDisposition);
    } else {
      headers.set(
        'content-disposition',
        `attachment; filename="world-${encodeURIComponent(jobId)}.mcworld"`
      );
    }

    const contentLength = res.headers.get('content-length');
    if (contentLength) {
      headers.set('content-length', contentLength);
    }

    // Stream the response body back to the client
    return new Response(res.body, {
      status: 200,
      headers,
    });
  } catch (err: unknown) {
    const message =
      err instanceof Error
        ? err.name === 'AbortError'
          ? 'Download request timed out'
          : err.message
        : 'Unknown error';
    return Response.json({ error: message }, { status: 502 });
  } finally {
    clearTimeout(timerId);
  }
}
