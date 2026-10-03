export type JobsStatus = { stage_bytes: number; backup_bytes: number };
export type CacheStatus = { total_bytes: number; file_count: number; publisher_bytes: number; core_bytes: number; removed_bytes?: number; jobs: JobsStatus };
export type Settings = { auto_cleanup: boolean; keep_backups: boolean; cache: CacheStatus };

// One binary-unit formatter shared by progress and cache rows.
export function formatSize(bytes: number) {
  if (!Number.isFinite(bytes)) return "—";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let n = Math.max(0, bytes), i = 0;
  while (n >= 1024 && i < units.length - 1) { n /= 1024; i++; }
  return `${i ? n.toFixed(1) : Math.round(n)} ${units[i]}`;
}

export function cacheSummary(status?: CacheStatus | null) {
  return `${formatSize(status?.total_bytes ?? 0)} · ${status?.file_count ?? 0} 个文件`;
}

export function cacheClearNote(removed: number) {
  return `已清理 ${formatSize(removed)} 下载缓存。`;
}

export function autoCleanupNote(enabled: boolean) {
  return enabled ? "已开启安装成功后自动清理下载缓存。" : "已关闭自动清理，缓存将保留直至手动清理。";
}
