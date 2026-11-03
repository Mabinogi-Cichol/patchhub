export type UpdateExecution = { state: string; stage?: string; error?: string; percent?: number;
  version?: string; restart?: string; restart_error?: string; restart_context?: number };
export const updateExecuting = (execution?: UpdateExecution | null) => ['preparing', 'applying'].includes(execution?.state ?? '');

export function updateProgressLabel(execution: UpdateExecution): string {
  const labels: Record<string, string> = {
    downloading: '下载中', validating: '校验中', preparing: '准备安装', backing_up: '备份中',
    installing: '安装中', verifying: '验证中', recovering: '恢复中', cleanup: '清理中', restarting: '正在重启客户端',
  };
  return `${Math.floor(execution.percent ?? 0)}% · ${labels[execution.stage ?? ''] ?? '准备更新'}`;
}
