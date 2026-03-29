'use client';

/**
 * ConversionControls
 *
 * Renders the action buttons (Convert, Fetch & Convert, Terrain Only) and
 * the no-file hint extracted from ExportPanel.
 */

type ConversionState = 'idle' | 'uploading' | 'converting' | 'done' | 'error';

interface ConversionControlsProps {
  conversionState: ConversionState;
  hasBbox: boolean;
  hasSourceFile: boolean;
  terrainOnly: boolean;
  onConvert: () => void;
  onFetchConvert: () => void;
  onTerrainConvert: () => void;
}

export function ConversionControls({
  conversionState,
  hasBbox,
  hasSourceFile,
  terrainOnly,
  onConvert,
  onFetchConvert,
  onTerrainConvert,
}: ConversionControlsProps) {
  const isIdle = conversionState === 'idle' || conversionState === 'error';

  if (!isIdle) return null;

  return (
    <>
      {/* Terrain-only button */}
      {hasBbox && terrainOnly && (
        <button
          onClick={onTerrainConvert}
          className="w-full rounded-lg py-2.5 text-sm font-bold transition-all"
          style={{
            background: 'linear-gradient(135deg, #4db8d4 0%, #2d8fa8 100%)',
            color: '#08090d',
            border: '1px solid rgba(77,184,212,0.7)',
            cursor: 'pointer',
            boxShadow: '0 0 16px rgba(77,184,212,0.25)',
            fontWeight: 700,
            letterSpacing: '0.02em',
          }}
        >
          Generate Terrain World
        </button>
      )}

      {/* Fetch & Convert button */}
      {hasBbox && !hasSourceFile && !terrainOnly && (
        <button
          onClick={onFetchConvert}
          className="w-full rounded-lg py-2.5 text-sm font-bold transition-all"
          style={{
            background: 'var(--accent-cyan, #4db8d4)',
            color: '#08090d',
            border: '1px solid rgba(77,184,212,0.7)',
            cursor: 'pointer',
            boxShadow: '0 0 16px rgba(77,184,212,0.25)',
            fontWeight: 700,
            letterSpacing: '0.02em',
          }}
        >
          Fetch &amp; Convert from Overpass
        </button>
      )}

      {/* Convert button */}
      <button
        onClick={onConvert}
        disabled={!hasSourceFile}
        className="w-full rounded-lg py-2.5 text-sm font-bold transition-all"
        style={
          hasSourceFile
            ? {
                background: 'var(--accent-gold)',
                color: '#08090d',
                border: '1px solid rgba(232,184,77,0.7)',
                cursor: 'pointer',
                boxShadow: '0 0 16px rgba(232,184,77,0.25)',
                fontWeight: 700,
                letterSpacing: '0.02em',
              }
            : {
                background: 'rgba(232,184,77,0.1)',
                color: 'rgba(232,184,77,0.35)',
                border: '1px solid rgba(232,184,77,0.15)',
                cursor: 'not-allowed',
              }
        }
      >
        Convert to .mcworld
      </button>

      {/* No-file hint */}
      {!hasSourceFile && (
        <p className="text-center text-xs" style={{ color: 'var(--text-muted)' }}>
          Upload a .osm.pbf file first
        </p>
      )}
    </>
  );
}
