// web/src/app/api/cache/route.ts
import { RUST_API_URL, TIMEOUTS } from '@/lib/api-config';

export async function GET(): Promise<Response> {
  try {
    const res = await fetch(`${RUST_API_URL}/cache/areas`, {
      signal: AbortSignal.timeout(TIMEOUTS.SHORT),
    });
    if (!res.ok) {
      return Response.json([], { status: 200 }); // degrade silently
    }
    const data: unknown = await res.json();
    return Response.json(data);
  } catch {
    return Response.json([]); // never error — return empty array
  }
}
