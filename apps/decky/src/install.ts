import { reason, type Patch, type Plan, type Transaction } from "@steam-patch/ui";

export type PreparedInstall = { archive: string; acquisition: "local" | "download"; mode?: "reviewed" | "publisher_root"; plan: Plan };

export function preparedSource(acquisition: PreparedInstall["acquisition"], mode?: PreparedInstall["mode"]): string {
  if (mode === "publisher_root") return `${acquisition === "local" ? "已复用本机官方下载缓存" : "已获取官方 ZIP"}，按游戏根目录生成计划。请核对文件变更后确认安装。此方式尚未逐游戏验证效果。`;
  return acquisition === "local"
    ? "已复用本地已审核 ZIP，并完成校验。请检查计划后确认安装。"
    : "已下载已审核 ZIP，并完成校验。请检查计划后确认安装。";
}

export function installBlocker(patch: Patch): string | null {
  if (patch.blockers.length) return patch.blockers.map(reason).join(" ");
  if (!["managed_zip", "publisher_zip"].includes(patch.support_level)) return "此补丁没有可用的 ZIP 安装方式，只能查看来源。";
  return null;
}

export function planExpired(plan: Plan, now = Date.now()): boolean {
  return !Number.isFinite(plan.expires_at) || !plan.expires_at || now >= plan.expires_at * 1000;
}

export function transactionResult(tx: Transaction): string {
  const details = tx.error ? ` 原因：${reason(tx.error)}` : "";
  switch (tx.state) {
    case "committed": return `安装完成，文件已写入。${details}`;
    case "rolled_back": return `安装未完成，已回滚文件。${details}`;
    case "needs_recovery": return `安装未完成，需要运行修复未完成的操作。${details}`;
    default: return `安装返回状态：${tx.state}。请检查补丁操作记录。${details}`;
  }
}
