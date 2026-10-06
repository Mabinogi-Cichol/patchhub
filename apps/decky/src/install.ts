import { reason, type Transaction } from "@steam-patch/ui";

export function transactionResult(tx: Transaction): string {
  const details = tx.error ? ` 原因：${reason(tx.error)}` : "";
  switch (tx.state) {
    case "committed": return `安装完成，文件已写入。${details}`;
    case "rolled_back": return `安装未完成，已回滚文件。${details}`;
    case "needs_recovery": return `安装未完成，需要运行修复未完成的操作。${details}`;
    default: return `安装返回状态：${tx.state}。请检查补丁操作记录。${details}`;
  }
}
