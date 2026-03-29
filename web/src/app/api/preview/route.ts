import { RUST_API_URL, TIMEOUTS } from '@/lib/api-config';
const FETCH_TIMEOUT_MS = TIMEOUTS.CONVERT;

export async function POST(request: Request): Promise<Response> {
  let formData: FormData;
  try {
    formData = await request.formData();
  } catch {
    return Response.json({ error: 'Failed to parse form data' }, { status: 400 });
  }

  const file = formData.get('file');
  if (!file || !(file instanceof File)) {
    return Response.json({ error: 'No file field found in form data' }, { status: 400 });
  }

  // Forward multipart to Rust API /preview
  const forwardForm = new FormData();
  forwardForm.append('file', file);

  // Forward any additional form fields (e.g. options)
  for (const [key, value] of formData.entries()) {
    if (key !== 'file') {
      forwardForm.append(key, value);
    }
  }

  const controller = new AbortController();
  const timerId = setTimeout(() => controller.abort(), FETCH_TIMEOUT_MS);

  try {
    const res = await fetch(`${RUST_API_URL}/preview`, {
      method: 'POST',
      body: forwardForm,
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
          ? 'Preview request timed out'
          : err.message
        : 'Unknown error';
    return Response.json({ error: message }, { status: 502 });
  } finally {
    clearTimeout(timerId);
  }
}
