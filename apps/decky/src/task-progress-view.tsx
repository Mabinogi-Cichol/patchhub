import { ButtonItem, ModalRoot, ProgressBar } from "@decky/ui";
import { reason } from "@steam-patch/ui";
import { formatSize } from "./cache";
import { taskIsTerminal, type TaskSnapshot } from "./task-controller";
import { taskProgressPercent } from "./task-progress";

const stageNames: Record<string, string> = {
  scanning: "查找本地 ZIP", searching: "查找本地 ZIP", connecting: "正在连接", downloading: "下载补丁",
  validating: "校验文件", planning: "生成安装计划", backing_up: "备份原文件",
  writing: "写入文件", committing: "提交安装", rolling_back: "恢复原文件",
};

function duration(seconds: number) { return `${Math.floor(seconds / 60)} 分 ${Math.floor(seconds % 60)} 秒`; }

const styles = `
  .DialogContent.patch-task-modal { box-sizing:border-box; width:min(640px,calc(100vw - 96px)); max-width:100%; min-height:0; max-height:calc(100vh - 144px); padding:16px 20px; }
  .patch-task-shell { box-sizing:border-box; display:flex; flex-direction:column; min-height:0; max-height:min(520px,calc(100vh - 184px)); overflow:hidden; }
  .patch-task-header, .patch-task-footer { flex:none; }
  .patch-task-header h2 { margin:0 0 6px; font-size:22px; }
  .patch-task-header { padding-bottom:8px; }
  .patch-task-body { flex:1 1 auto; min-height:0; overflow-y:auto; overscroll-behavior:contain; padding-right:6px; }
  .patch-task-shell p { margin:6px 0; line-height:1.35; overflow-wrap:anywhere; }
  .patch-task-stage { margin:0 0 8px; }
  .patch-task-metrics { display:grid; grid-template-columns:repeat(2,minmax(0,1fr)); gap:4px 16px; margin:12px 0; }
  .patch-task-metrics span { min-width:0; overflow-wrap:anywhere; }
  .patch-task-warning { color:#ffb28d; }
  .patch-task-footer { padding-top:10px; }
  @media (max-width:600px) { .patch-task-metrics { grid-template-columns:minmax(0,1fr); } }
`;

export function TaskProgressView({ task, game, patch, onCancel }: {
  task: TaskSnapshot | null;
  game: string;
  patch: string;
  onCancel: () => void;
}) {
  const busy = task != null && !taskIsTerminal(task.state);
  const stage = task?.state === "cancelling" ? "正在取消" : task?.state === "rolling_back" ? "正在恢复原文件" : stageNames[task?.stage ?? ""] ?? task?.stage ?? "正在启动";
  const progress = taskProgressPercent(task);
  return <ModalRoot className="patch-task-modal" bDisableBackgroundDismiss bHideCloseIcon onCancel={onCancel} closeModal={onCancel}>
    <style>{styles}</style>
    <div className="patch-task-shell">
      <header className="patch-task-header"><h2>{task?.kind === "install" ? "安装补丁" : "准备补丁"}</h2></header>
      <div className="patch-task-body">
        <p>{game} · {patch}</p>
        <p className="patch-task-stage">当前阶段：{stage}{progress != null ? ` · ${Math.round(progress)}%` : ""}</p>
        <ProgressBar indeterminate={progress == null} nProgress={progress ?? 0} />
        <div className="patch-task-metrics">
          <span>已处理：{formatSize(task?.bytes_done ?? 0)}{progress != null ? ` / ${formatSize(task!.total_bytes!)}` : " / 总量未知"}</span>
          <span>文件：{task?.files_done ?? 0}{task?.total_files != null ? ` / ${task.total_files}` : ""}</span>
          {task?.kind === "prepare" && task.stage === "downloading" && <span>下载速度：{formatSize(task.speed_bps)}/s</span>}
          {task?.kind === "prepare" && task.archive_bytes != null && task.stage !== "downloading" && <span>ZIP 大小：{formatSize(task.archive_bytes)}</span>}
          {task?.kind === "install" && task.archive_bytes != null && <span>安装包大小：{formatSize(task.archive_bytes)}</span>}
          <span>已用：{duration(task?.elapsed_seconds ?? 0)}</span>
          {task?.kind === "prepare" && task.stage === "downloading" && <span>预计剩余：{task.eta_seconds == null ? "估算中" : duration(task.eta_seconds)}</span>}
        </div>
        {task?.current_file && <p>当前文件：{task.current_file}</p>}
        {task?.error && <p className="patch-task-warning">{reason(task.error)}</p>}
        {task?.state === "cancelled" && <p>已安全取消。</p>}
        {task?.state === "completed" && <p>{task.kind === "install" ? "安装已完成。" : "准备完成。"}</p>}
        {busy && <p>退出此界面将中断任务。安装取消时会恢复本次修改的文件。</p>}
      </div>
      <footer className="patch-task-footer"><ButtonItem layout="below" bottomSeparator="none" onClick={onCancel}
        disabled={task?.state === "cancelling" || task?.state === "rolling_back"}>
        {busy ? task?.state === "cancelling" || task?.state === "rolling_back" ? stage : "取消并返回" : "返回补丁管理"}
      </ButtonItem></footer>
    </div>
  </ModalRoot>;
}
