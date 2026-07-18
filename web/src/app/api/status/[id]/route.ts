import { proxyToRust, TIMEOUTS } from '@/lib/api-config';

export async function GET(
  _request: Request,
  { params }: { params: Promise<{ id: string }> },
): Promise<Response> {
  const { id } = await params;

  if (!id) {
    return Response.json({ error: 'Missing job id' }, { status: 400 });
  }

  return proxyToRust(`/status/${encodeURIComponent(id)}`, {
    method: 'GET',
    timeoutMs: TIMEOUTS.SHORT,
    timeoutLabel: 'Status',
  });
}
