import { proxyToRust, TIMEOUTS } from '@/lib/api-config';

export async function POST(request: Request): Promise<Response> {
  let body: unknown;
  try {
    body = await request.json();
  } catch {
    return Response.json({ error: 'Invalid JSON body' }, { status: 400 });
  }

  return proxyToRust('/fetch-preview', {
    method: 'POST',
    body: JSON.stringify(body),
    headers: { 'Content-Type': 'application/json' },
    timeoutMs: TIMEOUTS.FETCH_CONVERT,
    timeoutLabel: 'Fetch-preview',
  });
}
