'use client';

/**
 * DownloadProgress
 *
 * Renders the conversion progress bar, download progress bar, done/download
 * link, and error state extracted from ExportPanel.
 */

import { Progress } from '@/components/ui/progress';
import { formatBytes } from '@/lib/geo';

type ConversionState = 'idle' | 'uploading' | 'converting' | 'done' | 'error';

interface DownloadProgressProps {
  conversionState: ConversionState;
  progress: number;
  message: string;
  downloadUrl: string | null;
  downloadFilename: string | null;
  error: string | null;
  downloadProgress: number;
  downloadTotal: number;
  isDownloading: boolean;
  onReset: () => void;
}

export function DownloadProgress({
  conversionState,
  progress,
  message,
  downloadUrl,
  downloadFilename,
  error,
  downloadProgress,
  downloadTotal,
  isDownloading,
  onReset,
}: DownloadProgressProps) {
  const isRunning = conversionState === 'uploading' || conversionState === 'converting';
  const isDone = conversionState === 'done';
  const progressValue = Math.round(progress * 100);

  return (
    <>
      {/* Conversion progress */}
      {isRunning && (
        <div className="flex flex-col gap-2">
          <Progress
            value={progressValue}
            className="w-full"
            style={
              {
                '--progress-indicator-color': 'var(--accent-gold)',
              } as React.CSSProperties
            }
          />
          <div className="flex items-center justify-between">
            <p className="text-xs" style={{ color: 'var(--text-secondary)' }}>
              {message || (conversionState === 'uploading' ? 'Uploading\u2026' : 'Converting\u2026')}
            </p>
            <p
              className="text-xs tabular-nums"
              style={{ color: 'var(--accent-gold)', fontFamily: "'JetBrains Mono', monospace" }}
            >
              {progressValue}%
            </p>
          </div>
        </div>
      )}

      {/* Download progress */}
      {isDownloading && (
        <div className="flex flex-col gap-2">
          <Progress
            value={downloadTotal > 0 ? Math.round((downloadProgress / downloadTotal) * 100) : 0}
            className="w-full"
            style={
              {
                '--progress-indicator-color': 'var(--accent-cyan, #4db8d4)',
              } as React.CSSProperties
            }
          />
          <div className="flex items-center justify-between">
            <p className="text-xs" style={{ color: 'var(--text-secondary)' }}>
              Downloading .mcworld\u2026
            </p>
            <p
              className="text-xs tabular-nums"
              style={{
                color: 'var(--accent-cyan, #4db8d4)',
                fontFamily: "'JetBrains Mono', monospace",
              }}
            >
              {formatBytes(downloadProgress)}
              {downloadTotal > 0 ? ` / ${formatBytes(downloadTotal)}` : ''}
            </p>
          </div>
        </div>
      )}

      {/* Done state */}
      {isDone && downloadUrl && !isDownloading && (
        <div className="flex flex-col gap-2">
          <a
            href={downloadUrl}
            download={downloadFilename}
            className="block w-full rounded-lg py-2.5 text-center text-sm font-bold transition-all"
            style={{
              background: 'var(--success)',
              color: '#fff',
              border: '1px solid rgba(76,175,80,0.6)',
              textDecoration: 'none',
              boxShadow: '0 0 14px rgba(76,175,80,0.25)',
            }}
          >
            Download .mcworld
          </a>
          <button
            onClick={onReset}
            className="w-full rounded-lg py-1.5 text-xs transition-all"
            style={{
              background: 'transparent',
              color: 'var(--text-muted)',
              border: '1px solid var(--border-subtle)',
              cursor: 'pointer',
            }}
          >
            Convert another
          </button>
        </div>
      )}

      {/* Error state */}
      {conversionState === 'error' && error && (
        <div className="flex flex-col gap-2">
          <div
            className="rounded-md px-3 py-2 text-xs leading-relaxed"
            style={{
              background: 'rgba(232,93,93,0.08)',
              border: '1px solid rgba(232,93,93,0.2)',
              color: '#ef9a9a',
            }}
          >
            <span style={{ color: 'var(--error)', fontWeight: 600 }}>Error: </span>
            {error}
          </div>
          <button
            onClick={onReset}
            className="w-full rounded-lg py-1.5 text-xs transition-all"
            style={{
              background: 'transparent',
              color: 'var(--text-secondary)',
              border: '1px solid var(--border-medium)',
              cursor: 'pointer',
            }}
          >
            Try again
          </button>
        </div>
      )}
    </>
  );
}
