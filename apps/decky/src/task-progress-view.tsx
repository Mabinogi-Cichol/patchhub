import { Focusable, ModalRoot, ProgressBar } from "@decky/ui";
import { useLayoutEffect, useRef, type ReactNode } from "react";
import { reason } from "@steam-patch/ui";
import { formatSize } from "./cache";
import { type TaskSnapshot } from "./task-controller";
import { taskProgressPercent } from "./task-progress";

const stageNames: Record<string, string> = {
  connecting: "正在连接", downloading: "下载补丁", validating: "校验文件",
  analyzing_installer: "分析安装包", extracting: "提取文件", planning: "检查安装条件",
  applying: "写入文件", rolling_back: "恢复原文件", cleanup: "清理临时文件",
};
const substageNames: Record<string, string> = { backing_up: "保护现有文件", writing: "写入文件", committing: "提交安装" };

function duration(seconds: number) { return `${Math.floor(seconds / 60)} 分 ${Math.floor(seconds % 60)} 秒`; }

const styles = `
  .DialogContent.patch-task-modal { box-sizing:border-box; width:min(640px,calc(100vw - 96px)); max-width:100%; min-height:0; max-height:calc(100vh - 144px); padding:16px 20px; }
  .patch-task-shell { box-sizing:border-box; display:flex; flex-direction:column; min-height:0; max-height:min(520px,calc(100vh - 184px)); overflow:hidden; }
  .patch-task-header { flex:none; }
  .patch-task-header h2 { margin:0 0 6px; font-size:22px; }
  .patch-task-header { padding-bottom:8px; }
  .patch-task-body { flex:1 1 auto; min-height:0; overflow-y:auto; overscroll-behavior:contain; padding-right:6px; }
  .patch-task-shell p { margin:6px 0; line-height:1.35; overflow-wrap:anywhere; }
  .patch-task-stage { margin:0 0 8px; }
  .patch-task-metrics { display:grid; grid-template-columns:repeat(2,minmax(0,1fr)); gap:4px 16px; margin:12px 0; }
  .patch-task-metrics span { min-width:0; overflow-wrap:anywhere; }
  .patch-task-warning { color:#ffb28d; }
  @media (max-width:600px) {
    .DialogContent.patch-task-modal { width:calc(100vw - 24px); padding:12px 14px; }
    .patch-task-metrics { grid-template-columns:minmax(0,1fr); }
  }
`;

function TaskPanel({ title, onClose, cancelDescription, children }: {
  title: string; onClose: () => void; cancelDescription: string; children: ReactNode;
}) {
  const focus = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => { focus.current?.focus(); }, []);
  return <ModalRoot className="patch-task-modal" onCancel={onClose} closeModal={onClose}
    onEscKeypress={onClose} bDisableBackgroundDismiss bHideCloseIcon>
    <style>{styles}</style>
    <Focusable className="patch-task-shell" ref={focus} tabIndex={-1} preferredFocus noFocusRing
      onCancelActionDescription={cancelDescription}
      onCancel={event => { event.stopPropagation(); onClose(); }}>
      <header className="patch-task-header"><h2>{title}</h2></header>
      <div className="patch-task-body">{children}</div>
    </Focusable>
  </ModalRoot>;
}

export function TaskResultView({ task, onClose }: { task: TaskSnapshot; onClose: () => void }) {
  const recovery = task.result?.state === "needs_recovery" || task.result?.recovery?.required === true || task.error === "task_interrupted";
  const message = recovery ? "安装已中断，仍需检查并恢复原文件。" : task.state === "completed" ? "安装已完成。"
    : task.state === "cancelled" ? "已安全取消。" : "安装失败。";
  const error = task.result?.recovery?.code ?? task.error ?? task.result?.error;
  return <TaskPanel title="安装结果" onClose={onClose} cancelDescription="关闭">
    <p>{message}</p>
    {error && <p className="patch-task-warning">{reason(error)}{task.result?.recovery?.detail ?? task.error_detail ? ` · ${task.result?.recovery?.detail ?? task.error_detail}` : ""}</p>}
  </TaskPanel>;
}

export function TaskProgressView({ task, game, patch, onClose }: {
  task: TaskSnapshot | null; game: string; patch: string; onClose: () => void;
}) {
  const stage = task?.state === "cancelling" ? "正在取消" : task?.state === "rolling_back" ? "正在恢复原文件" : stageNames[task?.stage ?? ""] ?? task?.stage ?? "正在启动";
  const substage = task?.stage === "applying" && task.substage ? ` · ${substageNames[task.substage] ?? task.substage}` : "";
  const progress = taskProgressPercent(task);
  return <TaskPanel title="安装补丁" onClose={onClose} cancelDescription="关闭并取消安装">
    <p>{game} · {patch}</p>
    <p className="patch-task-stage">当前阶段：{stage}{substage}{progress != null ? ` · ${Math.round(progress)}%` : ""}</p>
    <ProgressBar indeterminate={progress == null} nProgress={progress ?? 0} />
    <div className="patch-task-metrics">
      <span>已处理：{formatSize(task?.bytes_done ?? 0)}{progress != null ? ` / ${formatSize(task!.total_bytes!)}` : " / 总量未知"}</span>
      <span>文件：{task?.files_done ?? 0}{task?.total_files != null ? ` / ${task.total_files}` : ""}</span>
      {task?.stage === "downloading" && <span>下载速度：{formatSize(task.speed_bps)}/s</span>}
      {task?.archive_bytes != null && <span>安装包大小：{formatSize(task.archive_bytes)}</span>}
      <span>已用：{duration(task?.elapsed_seconds ?? 0)}</span>
      {task?.stage === "downloading" && <span>预计剩余：{task.eta_seconds == null ? "估算中" : duration(task.eta_seconds)}</span>}
    </div>
    {task?.current_file && <p>当前文件：{task.current_file}</p>}
    <p>关闭将取消安装，并恢复本次修改的文件。</p>
  </TaskPanel>;
}
