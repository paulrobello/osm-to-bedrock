'use client';

import React from 'react';
import { Sun, Moon, X } from 'lucide-react';
import { cn } from '@/lib/utils';

interface SidebarProps {
  children?: React.ReactNode;
  className?: string;
  theme?: 'dark' | 'light';
  onToggleTheme?: () => void;
  isMobile?: boolean;
  onClose?: () => void;
}

export function Sidebar({ children, className, theme, onToggleTheme, isMobile, onClose }: SidebarProps) {
  return (
    <aside
      className={cn(
        'flex h-full shrink-0 flex-col',
        isMobile ? 'w-full' : 'w-[320px]',
        className,
      )}
      style={{
        background: 'linear-gradient(180deg, var(--bg-surface) 0%, var(--bg-deep) 100%)',
        borderRight: isMobile ? 'none' : '1px solid var(--border-subtle)',
        borderTop: isMobile ? '1px solid var(--border-subtle)' : 'none',
      }}
    >
      {isMobile && (
        <div className="flex justify-center py-2">
          <div className="h-1 w-10 rounded-full" style={{ background: 'var(--border-medium)' }} />
        </div>
      )}
      {/* Header */}
      <div
        className="flex items-center gap-2.5 px-4 py-3.5"
        style={{ borderBottom: '1px solid var(--border-subtle)' }}
      >
        {/* Compass icon */}
        <svg
          width="16"
          height="16"
          viewBox="0 0 16 16"
          fill="none"
          xmlns="http://www.w3.org/2000/svg"
          style={{ flexShrink: 0 }}
          aria-hidden="true"
        >
          <circle cx="8" cy="8" r="7" stroke="#8b7cf6" strokeWidth="1.2" />
          <circle cx="8" cy="8" r="1.2" fill="#8b7cf6" />
          {/* North needle */}
          <polygon points="8,2.5 6.8,8 9.2,8" fill="#8b7cf6" opacity="0.9" />
          {/* South needle */}
          <polygon points="8,13.5 6.8,8 9.2,8" fill="#505870" />
          {/* Cardinal ticks */}
          <line x1="8" y1="1.2" x2="8" y2="2.0" stroke="#8b7cf6" strokeWidth="1" strokeLinecap="round" />
          <line x1="8" y1="14.0" x2="8" y2="14.8" stroke="#505870" strokeWidth="1" strokeLinecap="round" />
          <line x1="1.2" y1="8" x2="2.0" y2="8" stroke="#505870" strokeWidth="1" strokeLinecap="round" />
          <line x1="14.0" y1="8" x2="14.8" y2="8" stroke="#505870" strokeWidth="1" strokeLinecap="round" />
        </svg>

        <span
          className="text-[10px] font-semibold uppercase"
          style={{
            color: 'var(--accent-purple)',
            letterSpacing: '0.2em',
            fontFamily: "'DM Sans', system-ui, sans-serif",
            fontWeight: 600,
          }}
        >
          OSM Explorer
        </span>

        {onToggleTheme && (
          <button
            onClick={onToggleTheme}
            className="ml-auto rounded p-1 transition-colors"
            style={{
              color: 'var(--text-muted)',
              background: 'transparent',
              border: 'none',
              cursor: 'pointer',
            }}
            aria-label="Toggle theme"
          >
            {theme === 'dark' ? <Sun className="h-3.5 w-3.5" /> : <Moon className="h-3.5 w-3.5" />}
          </button>
        )}
        {isMobile && onClose && (
          <button
            onClick={onClose}
            className="rounded p-1 transition-colors"
            style={{
              color: 'var(--text-muted)',
              background: 'transparent',
              border: 'none',
              cursor: 'pointer',
              marginLeft: onToggleTheme ? undefined : 'auto',
            }}
            aria-label="Close sidebar"
          >
            <X className="h-4 w-4" />
          </button>
        )}
      </div>

      {/* Scrollable content */}
      <div
        className="flex-1 min-h-0 overflow-y-auto"
        style={{ scrollbarWidth: 'thin', scrollbarColor: 'rgba(255,255,255,0.1) transparent' }}
      >
        <div className="flex flex-col">{children}</div>
      </div>
    </aside>
  );
}
