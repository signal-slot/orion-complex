import type { EnvironmentState } from "./types";

/**
 * Format a byte count into a human-readable string (e.g. "4.00 GB").
 */
export function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  const k = 1024;
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  const value = bytes / Math.pow(k, i);
  return `${value.toFixed(i === 0 ? 0 : 2)} ${units[i]}`;
}

/**
 * Format a Unix timestamp (seconds) into a locale date-time string.
 */
export function formatDate(unix: number): string {
  return new Date(unix * 1000).toLocaleString();
}

/**
 * Return a human-readable relative time string from a Unix timestamp (seconds).
 */
export function relativeTime(unix: number): string {
  const now = Date.now() / 1000;
  const diff = now - unix;

  if (Math.abs(diff) < 5) return "just now";

  const absDiff = Math.abs(diff);
  const suffix = diff > 0 ? "ago" : "from now";

  if (absDiff < 60) return `${Math.floor(absDiff)}s ${suffix}`;
  if (absDiff < 3600) return `${Math.floor(absDiff / 60)}m ${suffix}`;
  if (absDiff < 86400) return `${Math.floor(absDiff / 3600)}h ${suffix}`;
  if (absDiff < 2592000) return `${Math.floor(absDiff / 86400)}d ${suffix}`;

  return formatDate(unix);
}

/**
 * Return Tailwind CSS classes for an environment state badge.
 */
export function stateColor(state: EnvironmentState | string | null): string {
  switch (state) {
    case "running":
      return "bg-green-100 text-green-800";
    case "creating":
    case "resuming":
    case "rebooting":
      return "bg-blue-100 text-blue-800";
    case "suspending":
    case "migrating":
    case "destroying":
      return "bg-yellow-100 text-yellow-800";
    case "capturing":
      return "bg-indigo-100 text-indigo-800";
    case "suspended":
      return "bg-gray-100 text-gray-800";
    case "failed":
      return "bg-red-100 text-red-800";
    default:
      return "bg-gray-100 text-gray-600";
  }
}
