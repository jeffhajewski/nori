import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

export function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(1))} ${sizes[i]}`;
}

export function formatNumber(num: number): string {
  if (num >= 1_000_000) {
    return `${(num / 1_000_000).toFixed(1)}M`;
  }
  if (num >= 1_000) {
    return `${(num / 1_000).toFixed(1)}K`;
  }
  return num.toString();
}

export function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  if (ms < 3600_000) return `${Math.floor(ms / 60_000)}m ${Math.floor((ms % 60_000) / 1000)}s`;
  return `${Math.floor(ms / 3600_000)}h ${Math.floor((ms % 3600_000) / 60_000)}m`;
}

export function formatTimestamp(ms: number): string {
  return new Date(ms).toLocaleTimeString("en-US", {
    hour12: false,
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

export function getHeatColor(heat: number): string {
  // Inferno color scale approximation
  if (heat < 0.25) return "hsl(240, 100%, 20%)";       // Dark blue
  if (heat < 0.5) return "hsl(280, 80%, 40%)";         // Purple
  if (heat < 0.75) return "hsl(30, 100%, 50%)";        // Orange
  return "hsl(50, 100%, 70%)";                          // Yellow
}

export function getLevelColor(level: number): string {
  const colors = [
    "hsl(0, 84%, 60%)",     // L0 - red
    "hsl(25, 95%, 53%)",    // L1 - orange
    "hsl(38, 92%, 50%)",    // L2 - amber
    "hsl(48, 96%, 53%)",    // L3 - yellow
    "hsl(142, 71%, 45%)",   // L4 - green
    "hsl(172, 66%, 50%)",   // L5 - teal
    "hsl(217, 91%, 60%)",   // L6 - blue
  ];
  return colors[Math.min(level, 6)];
}
