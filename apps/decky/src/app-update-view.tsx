import { DialogButton, Focusable, ModalRoot, ProgressBar } from '@decky/ui';
import { useLayoutEffect, useRef } from 'react';
import { formatSize } from './cache';

export type UpdateExecution = { state: string; stage?: string; error?: string; bytes?: number; total?: number; file?: string };
export const updateExecuting = (execution?: UpdateExecution | null) => ['preparing', 'applying'].includes(execution?.state ?? '');

export function AppUpdateProgress({ execution }: { execution: UpdateExecution }) {
  const labels: Record<string, string> = {
    downloading: '下载更新包', validating: '校验更新包', preparing: '准备安装',
    backing_up: '备份当前插件', installing: '安装新版插件', verifying: '验证新版插件',
    recovering: '恢复安装前状态', cleanup: '清理更新材料',
  };
  const downloading = execution.stage === 'downloading' || (!execution.stage && !!execution.file);
  const percent = downloading && execution.total && execution.total > 0
    ? Math.min(100, (execution.bytes ?? 0) / execution.total * 100) : null;
  const stage = downloading ? labels.downloading : labels[execution.stage ?? ''] ?? (execution.state === 'applying' ? labels.installing : '准备下载更新包');
  return <div style={{ minWidth: 0, overflowWrap: 'anywhere' }}>
    <p role="status">{stage}{percent != null ? ` · ${Math.floor(percent)}%` : ''}</p>
    <ProgressBar indeterminate={percent == null} nProgress={percent ?? 0} />
    {downloading && <p>{formatSize(execution.bytes ?? 0)} / {execution.total ? formatSize(execution.total) : '总量未知'}</p>}
    {execution.file && <p>{execution.file}</p>}
    <p>{execution.state === 'preparing' ? '下载完成并校验后将自动安装。' : '请勿关闭 Steam。更新完成后，请退出当前页面并重新打开 PatchHub。'}</p>
  </div>;
}

export function AppUpdateDialogView({ execution, error, actionError, busy, onCancelDownload, onClose }: {
  execution?: UpdateExecution | null; error?: string; actionError?: string; busy: boolean; onCancelDownload: () => void; onClose: () => void;
}) {
  const focus = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => { focus.current?.focus(); }, []);
  const executing = updateExecuting(execution);
  return <ModalRoot className="patch-update-modal" onCancel={onClose} closeModal={onClose}
    onEscKeypress={onClose} bDisableBackgroundDismiss bHideCloseIcon>
    <style>{`.DialogContent.patch-update-modal { box-sizing:border-box; width:min(600px,calc(100vw - 48px)); max-width:100%; min-height:0; max-height:calc(100vh - 96px); padding:20px; }
      .patch-update-body { max-height:calc(100vh - 156px); overflow-y:auto; overflow-wrap:anywhere; }
      .patch-update-body h2 { margin:0 0 12px; font-size:22px; }
      .patch-update-body p { margin:10px 0; line-height:1.4; }
      .patch-update-actions { display:flex; flex-wrap:wrap; gap:12px; margin-top:16px; }
      .patch-update-actions button { flex:1 1 180px; min-width:0; min-height:44px; }`}</style>
    <Focusable ref={focus} className="patch-update-body" tabIndex={-1} preferredFocus noFocusRing
      onCancelActionDescription="收起更新进度" onCancel={event => { event.stopPropagation(); onClose(); }}>
      <h2>应用更新</h2>
      {executing && <AppUpdateProgress execution={execution!} />}
      {execution?.state === 'completed' && <p role="status">更新已完成。请退出当前插件页面，再从 Decky 重新打开 PatchHub，以加载新版界面。</p>}
      {execution?.state === 'failed' && <p role="alert">更新未完成：{execution.error}</p>}
      {actionError && <p role="alert">取消下载失败：{actionError}</p>}
      {error && <p role="status">{executing ? '正在等待插件重载或重新连接，更新结果以安装器最终状态为准。' : '读取更新状态失败。'} {error}</p>}
      {executing && <p>收起后可在全局设置中查看更新进度。</p>}
      <div className="patch-update-actions">
        {execution?.state === 'preparing' && <DialogButton disabled={busy} onClick={onCancelDownload}>取消下载</DialogButton>}
        <DialogButton onClick={onClose}>{executing ? '收起' : '关闭'}</DialogButton>
      </div>
    </Focusable>
  </ModalRoot>;
}
