import type * as GeoJSON from 'geojson';

export const API_URL = process.env.NEXT_PUBLIC_API_URL || 'http://localhost:3002';

const DEFAULT_TIMEOUT_MS = 10_000;

/**
 * Creates a fetch request with a timeout signal.
 */
function fetchWithTimeout(
  url: string,
  options: RequestInit = {},
  timeoutMs: number = DEFAULT_TIMEOUT_MS
): Promise<Response> {
  const controller = new AbortController();
  const id = setTimeout(() => controller.abort(), timeoutMs);
  return fetch(url, { ...options, signal: controller.signal }).finally(() =>
    clearTimeout(id)
  );
}

export interface ParsePBFResult {
  geojson: GeoJSON.FeatureCollection;
  bounds: number[];
  stats: object;
}

/**
 * Sends a .osm.pbf file to the Rust API /parse endpoint and returns parsed GeoJSON.
 */
export async function parsePBF(file: File): Promise<ParsePBFResult> {
  const form = new FormData();
  form.append('file', file);

  const res = await fetchWithTimeout(`${API_URL}/parse`, {
    method: 'POST',
    body: form,
  });

  if (!res.ok) {
    const text = await res.text().catch(() => res.statusText);
    throw new Error(`parsePBF failed (${res.status}): ${text}`);
  }

  return res.json() as Promise<ParsePBFResult>;
}

export interface ConversionJob {
  job_id: string;
}

/**
 * Starts a conversion job on the Rust API.
 */
export async function startConversion(formData: FormData): Promise<ConversionJob> {
  const res = await fetchWithTimeout(`${API_URL}/convert`, {
    method: 'POST',
    body: formData,
  });

  if (!res.ok) {
    const text = await res.text().catch(() => res.statusText);
    throw new Error(`startConversion failed (${res.status}): ${text}`);
  }

  return res.json() as Promise<ConversionJob>;
}

export interface JobStatus {
  state: string;
  progress: number;
  message: string;
}

/**
 * Polls job status from the Rust API.
 */
export async function getStatus(jobId: string): Promise<JobStatus> {
  const res = await fetchWithTimeout(`${API_URL}/status/${encodeURIComponent(jobId)}`, {
    method: 'GET',
  });

  if (!res.ok) {
    const text = await res.text().catch(() => res.statusText);
    throw new Error(`getStatus failed (${res.status}): ${text}`);
  }

  return res.json() as Promise<JobStatus>;
}

/**
 * Returns the download URL for a completed conversion job.
 */
export function getDownloadUrl(jobId: string): string {
  return `${API_URL}/download/${encodeURIComponent(jobId)}`;
}
