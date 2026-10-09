export type CacheStatus = { performed: boolean; removed_bytes: number };
export type Settings = { schema_version: number; auto_cleanup: boolean; cache: CacheStatus | null };

// One binary-unit formatter shared by progress and cache rows.
export function formatSize(bytes: number) {
  if (!Number.isFinite(bytes)) return "—";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let n = Math.max(0, bytes), i = 0;
  while (n >= 1024 && i < units.length - 1) { n /= 1024; i++; }
  return `${i ? n.toFixed(1) : Math.round(n)} ${units[i]}`;
}

export function cacheSummary() {
  return "当前版本不提供占用统计";
}

export function cacheClearNote(removed: number, performed = true) {
  return performed ? `已清理 ${formatSize(removed)} 下载缓存。` : "当前没有可清理的下载缓存。";
}

export function autoCleanupNote(enabled: boolean) {
  return enabled ? "已开启安装成功后自动清理下载缓存。" : "已关闭自动清理，缓存将保留直至手动清理。";
}
