/**
 * Tests for the useConversion polling state machine (ARC-009).
 *
 * Scope: idle → uploading → converting → done/error transitions, polling
 * cadence, terminal-state exit, and manual cleanup via reset().
 *
 * The hook's unmount cleanup gap (no `useEffect` cleanup, so the poll timer
 * continues to fire after unmount) is QA-010 and tracked by the skipped test
 * at the bottom of this file — it is ready to enable once that fix lands.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { renderHook, act } from '@testing-library/react';
import { useConversion } from './useConversion';
import type { ConvertOptions } from './useConversion';

const DEFAULT_OPTIONS: ConvertOptions = {
  worldName: 'TestWorld',
  scale: 1,
  buildingHeight: 1,
  seaLevel: 65,
};

const POLL_INTERVAL_MS = 2_000;

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function makeFetchMock(handlers: {
  onStart?: () => Response | Promise<Response>;
  onStatus?: (callIndex: number) => Response | Promise<Response>;
  onDownload?: () => Response | Promise<Response>;
}): typeof fetch {
  let statusCalls = 0;
  return vi.fn(async (url: RequestInfo | URL) => {
    const u = String(url);
    if (u === '/api/fetch-convert') return await handlers.onStart!();
    if (u.startsWith('/api/status/')) {
      statusCalls += 1;
      return await handlers.onStatus!(statusCalls);
    }
    if (u.startsWith('/api/download')) return await handlers.onDownload!();
    return new Response('not found', { status: 404 });
  }) as unknown as typeof fetch;
}

describe('useConversion', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('starts in the idle state with no progress, error, or download URL', () => {
    vi.stubGlobal('fetch', vi.fn());
    const { result } = renderHook(() => useConversion());
    expect(result.current.conversionState).toBe('idle');
    expect(result.current.progress).toBe(0);
    expect(result.current.error).toBeNull();
    expect(result.current.downloadUrl).toBeNull();
    expect(result.current.downloadFilename).toBe('world.mcworld');
  });

  it('transitions uploading → converting once the start endpoint returns a job_id', async () => {
    const fetchMock = makeFetchMock({
      onStart: () => jsonResponse({ job_id: 'job-1' }),
      onStatus: () => jsonResponse({ state: 'converting', progress: 0, message: '' }),
      onDownload: () => new Response('', { status: 200 }),
    });
    vi.stubGlobal('fetch', fetchMock);

    const { result } = renderHook(() => useConversion());
    await act(async () => {
      await result.current.startFetchConvert([0, 0, 1, 1], DEFAULT_OPTIONS);
    });

    expect(result.current.conversionState).toBe('converting');
    expect(result.current.status).toBe('converting');
    expect(fetchMock).toHaveBeenCalledOnce();
    expect(String(vi.mocked(fetchMock).mock.calls[0]![0])).toBe('/api/fetch-convert');
  });

  it('does not poll before the first POLL_INTERVAL_MS elapses', async () => {
    const fetchMock = makeFetchMock({
      onStart: () => jsonResponse({ job_id: 'job-pace' }),
      onStatus: () => jsonResponse({ state: 'converting', progress: 10, message: 'tick' }),
      onDownload: () => new Response('', { status: 200 }),
    });
    vi.stubGlobal('fetch', fetchMock);

    const { result } = renderHook(() => useConversion());
    await act(async () => {
      await result.current.startFetchConvert([0, 0, 1, 1], DEFAULT_OPTIONS);
    });

    // Just under one interval — no status call yet.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(POLL_INTERVAL_MS - 1);
    });
    expect(fetchMock).toHaveBeenCalledOnce();
  });

  it('polls at POLL_INTERVAL_MS and reflects progress + message from the status endpoint', async () => {
    const fetchMock = makeFetchMock({
      onStart: () => jsonResponse({ job_id: 'job-2' }),
      onStatus: (i) => jsonResponse({
        state: 'converting',
        progress: 25 * i,
        message: `Step ${i}`,
      }),
      onDownload: () => new Response('', { status: 200 }),
    });
    vi.stubGlobal('fetch', fetchMock);

    const { result } = renderHook(() => useConversion());
    await act(async () => {
      await result.current.startFetchConvert([0, 0, 1, 1], DEFAULT_OPTIONS);
    });

    await act(async () => {
      await vi.advanceTimersByTimeAsync(POLL_INTERVAL_MS);
    });
    expect(result.current.conversionState).toBe('converting');
    expect(result.current.progress).toBe(25);
    expect(result.current.message).toBe('Step 1');

    await act(async () => {
      await vi.advanceTimersByTimeAsync(POLL_INTERVAL_MS);
    });
    expect(result.current.progress).toBe(50);
    expect(result.current.message).toBe('Step 2');
  });

  it('terminates on state=done and stops further polling', async () => {
    const fetchMock = makeFetchMock({
      onStart: () => jsonResponse({ job_id: 'job-3' }),
      onStatus: (i) => i === 1
        ? jsonResponse({ state: 'converting', progress: 50, message: 'Half' })
        : jsonResponse({ state: 'done', progress: 100, message: 'Done' }),
      onDownload: () => new Response('world-bytes', { status: 200 }),
    });
    vi.stubGlobal('fetch', fetchMock);

    const { result } = renderHook(() => useConversion());
    await act(async () => {
      await result.current.startFetchConvert([0, 0, 1, 1], DEFAULT_OPTIONS);
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(POLL_INTERVAL_MS);
    });
    expect(result.current.conversionState).toBe('converting');

    await act(async () => {
      await vi.advanceTimersByTimeAsync(POLL_INTERVAL_MS);
    });
    expect(result.current.conversionState).toBe('done');
    expect(result.current.progress).toBe(100);

    // 'done' is terminal — no more status polls fire.
    const callsAfterDone = vi.mocked(fetchMock).mock.calls.length;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(POLL_INTERVAL_MS * 3);
    });
    expect(vi.mocked(fetchMock).mock.calls.length).toBe(callsAfterDone);
  });

  it('treats state=complete and state=completed as terminal success', async () => {
    const fetchMock = makeFetchMock({
      onStart: () => jsonResponse({ job_id: 'job-complete' }),
      onStatus: () => jsonResponse({ state: 'complete', progress: 100, message: 'done' }),
      onDownload: () => new Response('', { status: 200 }),
    });
    vi.stubGlobal('fetch', fetchMock);

    const { result } = renderHook(() => useConversion());
    await act(async () => {
      await result.current.startFetchConvert([0, 0, 1, 1], DEFAULT_OPTIONS);
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(POLL_INTERVAL_MS);
    });
    expect(result.current.conversionState).toBe('done');
  });

  it('terminates on state=failed with the upstream message in error', async () => {
    const fetchMock = makeFetchMock({
      onStart: () => jsonResponse({ job_id: 'job-4' }),
      onStatus: () => jsonResponse({ state: 'failed', progress: 30, message: 'boom' }),
      onDownload: () => new Response('', { status: 200 }),
    });
    vi.stubGlobal('fetch', fetchMock);

    const { result } = renderHook(() => useConversion());
    await act(async () => {
      await result.current.startFetchConvert([0, 0, 1, 1], DEFAULT_OPTIONS);
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(POLL_INTERVAL_MS);
    });

    expect(result.current.conversionState).toBe('error');
    expect(result.current.error).toBe('boom');
    expect(result.current.progress).toBe(30);
  });

  it('transitions to error when the start endpoint returns non-2xx', async () => {
    const fetchMock = makeFetchMock({
      onStart: () => jsonResponse({ error: 'bad bbox' }, 400),
      onStatus: () => jsonResponse({ state: 'converting', progress: 0, message: '' }),
      onDownload: () => new Response('', { status: 200 }),
    });
    vi.stubGlobal('fetch', fetchMock);

    const { result } = renderHook(() => useConversion());
    await act(async () => {
      await result.current.startFetchConvert([0, 0, 1, 1], DEFAULT_OPTIONS);
    });

    expect(result.current.conversionState).toBe('error');
    expect(result.current.error).toBe('bad bbox');
  });

  it('transitions to error when the start endpoint response is missing job_id', async () => {
    const fetchMock = makeFetchMock({
      onStart: () => jsonResponse({ unrelated: 'field' }),
      onStatus: () => jsonResponse({ state: 'converting', progress: 0, message: '' }),
      onDownload: () => new Response('', { status: 200 }),
    });
    vi.stubGlobal('fetch', fetchMock);

    const { result } = renderHook(() => useConversion());
    await act(async () => {
      await result.current.startFetchConvert([0, 0, 1, 1], DEFAULT_OPTIONS);
    });

    expect(result.current.conversionState).toBe('error');
    expect(result.current.error).toContain('job_id');
  });

  it('transitions to error when a status poll itself rejects', async () => {
    const fetchMock = makeFetchMock({
      onStart: () => jsonResponse({ job_id: 'job-net' }),
      onStatus: () => new Response('gateway', { status: 502 }),
      onDownload: () => new Response('', { status: 200 }),
    });
    vi.stubGlobal('fetch', fetchMock);

    const { result } = renderHook(() => useConversion());
    await act(async () => {
      await result.current.startFetchConvert([0, 0, 1, 1], DEFAULT_OPTIONS);
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(POLL_INTERVAL_MS);
    });

    expect(result.current.conversionState).toBe('error');
    expect(result.current.error).toContain('502');
  });

  it('reset() stops the poll timer and returns the hook to idle', async () => {
    const fetchMock = makeFetchMock({
      onStart: () => jsonResponse({ job_id: 'job-5' }),
      onStatus: () => jsonResponse({ state: 'converting', progress: 50, message: 'half' }),
      onDownload: () => new Response('', { status: 200 }),
    });
    vi.stubGlobal('fetch', fetchMock);

    const { result } = renderHook(() => useConversion());
    await act(async () => {
      await result.current.startFetchConvert([0, 0, 1, 1], DEFAULT_OPTIONS);
    });

    await act(async () => {
      result.current.reset();
    });

    expect(result.current.conversionState).toBe('idle');
    expect(result.current.progress).toBe(0);
    expect(result.current.error).toBeNull();
    expect(result.current.downloadUrl).toBeNull();

    const statusCallsBefore = vi.mocked(fetchMock).mock.calls.filter(
      (c) => String(c[0]).startsWith('/api/status/'),
    ).length;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(POLL_INTERVAL_MS * 5);
    });
    const statusCallsAfter = vi.mocked(fetchMock).mock.calls.filter(
      (c) => String(c[0]).startsWith('/api/status/'),
    ).length;
    expect(statusCallsAfter).toBe(statusCallsBefore);
  });

  it('reset() aborts the in-flight upload so the late response does not flip state', async () => {
    // The start POST never resolves on its own — only abort can end it.
    const fetchMock = vi.fn((_url: RequestInfo | URL, init?: RequestInit) => {
      const u = String(_url);
      if (u === '/api/fetch-convert') {
        return new Promise<Response>((_resolve, reject) => {
          const signal = init?.signal;
          if (signal?.aborted) {
            reject(Object.assign(new Error('aborted'), { name: 'AbortError' }));
            return;
          }
          signal?.addEventListener('abort', () => {
            reject(Object.assign(new Error('aborted'), { name: 'AbortError' }));
          });
        });
      }
      return Promise.resolve(new Response('', { status: 200 }));
    }) as unknown as typeof fetch;
    vi.stubGlobal('fetch', fetchMock);

    const { result } = renderHook(() => useConversion());

    let uploadPromise!: Promise<void>;
    act(() => {
      uploadPromise = result.current.startFetchConvert([0, 0, 1, 1], DEFAULT_OPTIONS);
    });

    await act(async () => {
      result.current.reset();
      await uploadPromise;
    });

    expect(result.current.conversionState).toBe('idle');
    expect(result.current.error).toBeNull();
  });

  // ---------------------------------------------------------------------------
  // QA-010 lives here. The hook does not register a useEffect cleanup, so on
  // unmount the poll timer keeps firing and calls setState on an unmounted
  // component. Skipping until that fix lands — at which point this test should
  // pass as written (flip `it.skip` to `it`).
  // ---------------------------------------------------------------------------
  it.skip('QA-010: stops polling when the component unmounts', async () => {
    const fetchMock = makeFetchMock({
      onStart: () => jsonResponse({ job_id: 'job-unmount' }),
      onStatus: () => jsonResponse({ state: 'converting', progress: 50, message: 'half' }),
      onDownload: () => new Response('', { status: 200 }),
    });
    vi.stubGlobal('fetch', fetchMock);

    const { result, unmount } = renderHook(() => useConversion());
    await act(async () => {
      await result.current.startFetchConvert([0, 0, 1, 1], DEFAULT_OPTIONS);
    });
    unmount();

    const statusCallsBefore = vi.mocked(fetchMock).mock.calls.filter(
      (c) => String(c[0]).startsWith('/api/status/'),
    ).length;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(POLL_INTERVAL_MS * 5);
    });
    const statusCallsAfter = vi.mocked(fetchMock).mock.calls.filter(
      (c) => String(c[0]).startsWith('/api/status/'),
    ).length;
    expect(statusCallsAfter).toBe(statusCallsBefore);
  });
});
