"use client";

import { useWsStatus, useStats, useClusterHealth, useIsLive } from "@/stores/eventStore";
import { formatNumber, formatTimestamp } from "@/lib/utils";
import { cn } from "@/lib/utils";

export function Header() {
  const wsStatus = useWsStatus();
  const stats = useStats();
  const health = useClusterHealth();
  const isLive = useIsLive();

  return (
    <header className="fixed right-0 top-0 z-30 ml-64 h-16 w-[calc(100%-16rem)] border-b border-border bg-background/80 backdrop-blur-sm">
      <div className="flex h-full items-center justify-between px-6">
        {/* Left side - Status indicators */}
        <div className="flex items-center gap-6">
          {/* Connection Status */}
          <div className="flex items-center gap-2">
            <div
              className={cn(
                "h-2.5 w-2.5 rounded-full transition-colors",
                wsStatus === "connected" && "bg-status-healthy animate-pulse-subtle",
                wsStatus === "connecting" && "bg-status-warning animate-pulse",
                wsStatus === "reconnecting" && "bg-status-warning animate-pulse",
                wsStatus === "disconnected" && "bg-status-critical"
              )}
            />
            <span className="text-sm text-muted-foreground">
              {wsStatus === "connected" && "Connected"}
              {wsStatus === "connecting" && "Connecting..."}
              {wsStatus === "reconnecting" && "Reconnecting..."}
              {wsStatus === "disconnected" && "Disconnected"}
            </span>
          </div>

          {/* Cluster Health */}
          <div className="flex items-center gap-2 border-l border-border pl-6">
            <div
              className={cn(
                "h-2.5 w-2.5 rounded-full",
                health.healthy && "bg-status-healthy",
                !health.healthy && health.dead === 0 && "bg-status-warning",
                health.dead > 0 && "bg-status-critical"
              )}
            />
            <span className="text-sm text-muted-foreground">
              {health.total > 0 ? (
                <>
                  {health.alive}/{health.total} nodes
                  {health.suspect > 0 && (
                    <span className="ml-1 text-status-warning">({health.suspect} suspect)</span>
                  )}
                  {health.dead > 0 && (
                    <span className="ml-1 text-status-critical">({health.dead} dead)</span>
                  )}
                </>
              ) : (
                "No nodes"
              )}
            </span>
          </div>

          {/* Live indicator */}
          <div className="flex items-center gap-2 border-l border-border pl-6">
            <div
              className={cn(
                "h-2 w-2 rounded-full",
                isLive ? "bg-red-500 animate-pulse" : "bg-muted-foreground"
              )}
            />
            <span className="text-sm text-muted-foreground">
              {isLive ? "LIVE" : "Playback"}
            </span>
          </div>
        </div>

        {/* Right side - Stats */}
        <div className="flex items-center gap-6">
          {/* Event count */}
          <div className="text-right">
            <p className="text-xs text-muted-foreground">Events</p>
            <p className="text-sm font-medium text-foreground">
              {formatNumber(stats.totalEventsReceived)}
            </p>
          </div>

          {/* Buffer size */}
          <div className="text-right border-l border-border pl-6">
            <p className="text-xs text-muted-foreground">Buffer</p>
            <p className="text-sm font-medium text-foreground">
              {formatNumber(stats.bufferSize)} batches
            </p>
          </div>

          {/* Last update */}
          <div className="text-right border-l border-border pl-6">
            <p className="text-xs text-muted-foreground">Last Event</p>
            <p className="text-sm font-medium text-foreground">
              {stats.lastEventTimestamp > 0
                ? formatTimestamp(stats.lastEventTimestamp)
                : "--:--:--"}
            </p>
          </div>

          {/* Theme toggle */}
          <div className="border-l border-border pl-6">
            <ThemeToggle />
          </div>
        </div>
      </div>
    </header>
  );
}

function ThemeToggle() {
  const toggleTheme = () => {
    const html = document.documentElement;
    const current = html.classList.contains("dark") ? "dark" : "light";
    const next = current === "dark" ? "light" : "dark";
    html.classList.remove(current);
    html.classList.add(next);
    localStorage.setItem("theme", next);
  };

  return (
    <button
      onClick={toggleTheme}
      className="rounded-lg p-2 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
      title="Toggle theme"
    >
      {/* Sun icon (shown in dark mode) */}
      <svg
        className="h-5 w-5 hidden dark:block"
        fill="none"
        viewBox="0 0 24 24"
        stroke="currentColor"
      >
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={1.5}
          d="M12 3v1m0 16v1m9-9h-1M4 12H3m15.364 6.364l-.707-.707M6.343 6.343l-.707-.707m12.728 0l-.707.707M6.343 17.657l-.707.707M16 12a4 4 0 11-8 0 4 4 0 018 0z"
        />
      </svg>
      {/* Moon icon (shown in light mode) */}
      <svg
        className="h-5 w-5 block dark:hidden"
        fill="none"
        viewBox="0 0 24 24"
        stroke="currentColor"
      >
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={1.5}
          d="M20.354 15.354A9 9 0 018.646 3.646 9.003 9.003 0 0012 21a9.003 9.003 0 008.354-5.646z"
        />
      </svg>
    </button>
  );
}
