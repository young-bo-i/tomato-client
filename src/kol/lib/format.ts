// Shared formatting helpers for the KOL panels.

/// Render an ISO timestamp as a coarse "time ago" string in Chinese,
/// falling back to a date once it's older than a week. `null`/`undefined`
/// render as an em dash. Previously copy-pasted verbatim across several
/// stats/books panels.
export function formatRelative(iso: string | null | undefined): string {
  if (!iso) return "—";
  const dt = new Date(iso);
  const diff = Date.now() - dt.getTime();
  if (diff < 60_000) return "刚刚";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)} 分钟前`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)} 小时前`;
  if (diff < 7 * 86_400_000) return `${Math.floor(diff / 86_400_000)} 天前`;
  return dt.toLocaleDateString();
}
