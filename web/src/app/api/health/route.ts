// web/src/app/api/health/route.ts
import { RUST_API_URL, TIMEOUTS } from '@/lib/api-config';

export async function GET(): Promise<Response> {
  try {
    const res = await fetch(`${RUST_API_URL}/health`, {
      signal: AbortSignal.timeout(TIMEOUTS.SHORT),
    });
    if (!res.ok) {
      return Response.json({ status: 'error', overture_available: false }, { status: 200 });
    }
    const data: unknown = await res.json();
    return Response.json(data);
  } catch {
    return Response.json({ status: 'error', overture_available: false });
  }
}
