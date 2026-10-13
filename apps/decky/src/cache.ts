import type { CleanupResult } from "@steam-patch/ui";
export type { CleanupResult } from "@steam-patch/ui";
export type CacheStatus = CleanupResult;
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

export function cacheClearNote(result: CleanupResult) {
  const removed = formatSize(result.removed_bytes);
  const failures = result.failures?.map(failure => failure.detail || failure.code).filter(Boolean) ?? [];
  const retry = failures.length ? `失败原因：${failures.join("；")}。可重试清理。` : "可重试清理。";
  switch (result.status) {
    case "completed": return result.performed ? `已清理 ${removed} 下载缓存。` : "当前没有可清理的下载缓存。";
    case "partial": return `已清理 ${removed} 下载缓存，但部分清理失败。${retry}`;
    case "failed": return `清理失败，已清理 ${removed}。${retry}`;
    case "busy": return "缓存当前正被安装或恢复任务使用，请稍后重试。";
    case "not_requested": return "本次未执行缓存清理。";
  }
}

export function autoCleanupNote(enabled: boolean) {
  return enabled ? "已开启安装成功后自动清理下载缓存。" : "已关闭自动清理，缓存将保留直至手动清理。";
}
