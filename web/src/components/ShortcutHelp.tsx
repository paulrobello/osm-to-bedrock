'use client';

interface ShortcutHelpProps {
  onClose: () => void;
}

const SHORTCUTS = [
  { key: 'D', description: 'Toggle draw box mode' },
  { key: 'K', description: 'Toggle all layers' },
  { key: '/', description: 'Focus search bar' },
  { key: 'Esc', description: 'Cancel current mode' },
  { key: '?', description: 'Toggle this help' },
];

export function ShortcutHelp({ onClose }: ShortcutHelpProps) {
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center"
      style={{ background: 'rgba(0, 0, 0, 0.6)', backdropFilter: 'blur(4px)' }}
      onClick={onClose}
    >
      <div
        className="w-80 rounded-xl p-5"
        style={{
          background: 'var(--bg-elevated)',
          border: '1px solid var(--border-medium)',
          boxShadow: '0 20px 60px rgba(0, 0, 0, 0.5)',
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <p
          className="mb-4 text-[10px] font-semibold uppercase"
          style={{ color: 'var(--accent-gold)', letterSpacing: '0.18em' }}
        >
          Keyboard Shortcuts
        </p>
        <div className="flex flex-col gap-2">
          {SHORTCUTS.map((s) => (
            <div key={s.key} className="flex items-center justify-between">
              <span className="text-sm" style={{ color: 'var(--text-secondary)' }}>
                {s.description}
              </span>
              <kbd
                className="rounded px-2 py-0.5 text-xs font-mono"
                style={{
                  background: 'var(--bg-hover)',
                  border: '1px solid var(--border-medium)',
                  color: 'var(--text-primary)',
                }}
              >
                {s.key}
              </kbd>
            </div>
          ))}
        </div>
        <button
          onClick={onClose}
          className="mt-4 w-full rounded-lg py-1.5 text-xs"
          style={{
            background: 'transparent',
            color: 'var(--text-muted)',
            border: '1px solid var(--border-subtle)',
            cursor: 'pointer',
          }}
        >
          Close
        </button>
      </div>
    </div>
  );
}
