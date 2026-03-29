'use client';

import { useEffect, useState, useCallback } from 'react';

interface ShortcutActions {
  toggleDrawBox?: () => void;
  toggleAllLayers?: () => void;
  cancelMode?: () => void;
  focusSearch?: () => void;
}

export function useKeyboardShortcuts(actions: ShortcutActions) {
  const [showHelp, setShowHelp] = useState(false);

  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement).tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return;

      switch (e.key) {
        case 'd':
          actions.toggleDrawBox?.();
          break;
        case 'k':
          actions.toggleAllLayers?.();
          break;
        case 'Escape':
          if (showHelp) {
            setShowHelp(false);
          } else {
            actions.cancelMode?.();
          }
          break;
        case '/':
          e.preventDefault();
          actions.focusSearch?.();
          break;
        case '?':
          setShowHelp((prev) => !prev);
          break;
      }
    },
    [actions, showHelp],
  );

  useEffect(() => {
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [handleKeyDown]);

  return { showHelp, setShowHelp };
}
