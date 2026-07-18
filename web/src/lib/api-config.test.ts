/**
 * Tests for the centralised Rust API config + proxy helper (ARC-009).
 *
 * Covers: RUST_API_URL default + env override, TIMEOUTS constants, and the
 * `proxyToRust` success / upstream-error / network-error / timeout paths.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

describe('api-config', () => {
  beforeEach(() => {
    vi.resetModules();
    vi.stubGlobal('fetch', vi.fn());
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.unstubAllEnvs();
    vi.useRealTimers();
  });

  describe('RUST_API_URL', () => {
    it('defaults to localhost:3002 when the env var is unset', async () => {
      vi.stubEnv('RUST_API_URL', '');
      const { RUST_API_URL } = await import('./api-config');
      expect(RUST_API_URL).toBe('http://localhost:3002');
    });

    it('honours the RUST_API_URL env var when set', async () => {
      vi.stubEnv('RUST_API_URL', 'http://rust-api:9000');
      const { RUST_API_URL } = await import('./api-config');
      expect(RUST_API_URL).toBe('http://rust-api:9000');
    });
  });

  describe('TIMEOUTS', () => {
    it('exposes the six named budgets with the documented values', async () => {
      const { TIMEOUTS } = await import('./api-config');
      expect(TIMEOUTS).toEqual({
        SHORT: 10_000,
        UPLOAD: 30_000,
        CONVERT: 60_000,
        FETCH_CONVERT: 120_000,
        TERRAIN_CONVERT: 300_000,
        DOWNLOAD: 120_000,
      });
    });
  });

  describe('proxyToRust', () => {
    it('passes upstream JSON through on 2xx and targets RUST_API_URL+path', async () => {
      vi.stubEnv('RUST_API_URL', 'http://rust-api:3002');
      const fetchMock = vi.fn(async () => new Response(JSON.stringify({ job_id: 'abc' }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      })) as unknown as typeof fetch;
      vi.stubGlobal('fetch', fetchMock);

      const { proxyToRust } = await import('./api-config');
      const res = await proxyToRust('/convert', {
        method: 'POST',
        body: '{"x":1}',
        headers: { 'content-type': 'application/json' },
        timeoutMs: 5_000,
        timeoutLabel: 'Convert',
      });

      expect(res.status).toBe(200);
      expect(await res.json()).toEqual({ job_id: 'abc' });
      expect(fetchMock).toHaveBeenCalledOnce();
      const [url, init] = vi.mocked(fetchMock).mock.calls[0]!;
      expect(url).toBe('http://rust-api:3002/convert');
      expect(init?.method).toBe('POST');
      expect(init?.body).toBe('{"x":1}');
    });

    it('returns 502 with the Rust API error envelope on upstream non-2xx', async () => {
      vi.stubEnv('RUST_API_URL', 'http://rust-api:3002');
      vi.stubGlobal('fetch', vi.fn(async () => new Response('upstream said no', {
        status: 400,
      })) as unknown as typeof fetch);

      const { proxyToRust } = await import('./api-config');
      const res = await proxyToRust('/bad', {
        method: 'GET',
        timeoutMs: 5_000,
        timeoutLabel: 'Test',
      });

      expect(res.status).toBe(502);
      const body = (await res.json()) as { error: string };
      expect(body.error).toContain('400');
      expect(body.error).toContain('upstream said no');
    });

    it('returns 502 with the underlying error message when fetch rejects', async () => {
      vi.stubEnv('RUST_API_URL', 'http://rust-api:3002');
      vi.stubGlobal('fetch', vi.fn(async () => {
        throw new Error('ECONNREFUSED');
      }) as unknown as typeof fetch);

      const { proxyToRust } = await import('./api-config');
      const res = await proxyToRust('/net', {
        method: 'GET',
        timeoutMs: 5_000,
        timeoutLabel: 'Test',
      });

      expect(res.status).toBe(502);
      const body = (await res.json()) as { error: string };
      expect(body.error).toBe('ECONNREFUSED');
    });

    it('returns 502 with the labelled timeout message when the abort signal fires', async () => {
      vi.useFakeTimers();
      vi.stubEnv('RUST_API_URL', 'http://rust-api:3002');
      vi.stubGlobal('fetch', vi.fn((_url: RequestInfo | URL, init?: RequestInit) => new Promise<Response>(
        (_resolve, reject) => {
          const signal = init?.signal;
          if (signal?.aborted) {
            reject(Object.assign(new Error('aborted'), { name: 'AbortError' }));
            return;
          }
          signal?.addEventListener('abort', () => {
            reject(Object.assign(new Error('aborted'), { name: 'AbortError' }));
          });
        },
      )) as unknown as typeof fetch);

      const { proxyToRust } = await import('./api-config');
      const promise = proxyToRust('/slow', {
        method: 'GET',
        timeoutMs: 1_000,
        timeoutLabel: 'Convert',
      });
      await vi.advanceTimersByTimeAsync(1_000);
      const res = await promise;

      expect(res.status).toBe(502);
      const body = (await res.json()) as { error: string };
      expect(body.error).toBe('Convert request timed out');
    });

    it('passes a non-AbortError error message through unchanged (no timeout masking)', async () => {
      vi.stubEnv('RUST_API_URL', 'http://rust-api:3002');
      vi.stubGlobal('fetch', vi.fn(async () => {
        const err = new Error('JSON parse failed');
        err.name = 'SyntaxError';
        throw err;
      }) as unknown as typeof fetch);

      const { proxyToRust } = await import('./api-config');
      const res = await proxyToRust('/json-fail', {
        method: 'GET',
        timeoutMs: 5_000,
        timeoutLabel: 'Convert',
      });

      expect(res.status).toBe(502);
      const body = (await res.json()) as { error: string };
      expect(body.error).toBe('JSON parse failed');
    });
  });
});
