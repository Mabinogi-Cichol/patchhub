import type { CacheInventory, CleanupResult } from "@patchhub/ui";
export type { CacheInventory, CleanupResult } from "@patchhub/ui";
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

export function cacheSummary(value: CacheInventory) {
  return `${formatSize(value.total_bytes)}${value.diagnostics.length ? "（部分路径无法统计）" : ""}`;
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

export type { BackupInventory, BackupCleanupResult } from "@patchhub/ui";
import type { BackupInventory, BackupCleanupResult } from "@patchhub/ui";
export function backupSummary(value: BackupInventory) {
  return `${value.files} 个文件 · ${formatSize(value.total_bytes)}${value.failures.length ? "（部分路径无法统计）" : ""}`;
}
export function backupClearNote(value: BackupCleanupResult) {
  const deleted = `已删除 ${value.removed_files} 个备份文件，释放 ${formatSize(value.removed_bytes)}`;
  const detail = value.failures.map(f => f.detail || f.code).join("；");
  if (value.status === "completed") return value.removed_files ? `${deleted}。` : "当前没有需要清理的安装备份。";
  if (value.removed_files === 0 && (value.status === "busy" || value.failures.some(f => f.code === "installer_busy"))) return "安装任务进行中，请结束后重试。";
  return `${deleted}；${value.status === "partial" ? "部分清理失败" : "清理失败"}：${detail}。可重试清理。`;
}
