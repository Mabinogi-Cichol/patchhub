import { sourcePhaseLabel } from "@steam-patch/ui";
import type { MatchGamesResult, MatchUpdateEvent, ProjectInstancesResult, SourceSubscription, SourceUpdateResult } from "@steam-patch/ui";

// Host snapshots for the three v5 source task methods (update_source /
// match_games / rematch_game). Decky has no push channel; the frontend polls
// `source_task_status` and merges these snapshots into the scan rows.
export type SourceTaskKind = "update_source" | "match_games" | "rematch_game";
export type SourceTaskState = "running" | "cancelling" | "completed" | "failed" | "cancelled";

export type SourceTaskProgress = {
  phase: string | null;
  completed: number | null;
  total: number | null;
  running: boolean;
  source_id?: string | null;
  settings_revision?: number | null;
};

export type SourceTaskMatch = {
  appid: number;
  instance_id: string | null;
  status: string;
  candidates: MatchUpdateEvent["candidates"];
  diagnostics: MatchUpdateEvent["diagnostics"];
  settings_revision?: number | null;
};

export type SourceTaskSnapshot = {
  task_id: string;
  kind: SourceTaskKind;
  key: string;
  state: SourceTaskState;
  instances: string[];
  appid: number | null;
  progress: SourceTaskProgress | null;
  matches: Record<string, SourceTaskMatch>;
  result: SourceUpdateResult | MatchGamesResult | null;
  error: string | null;
  error_detail?: string | null;
  started_at?: number;
};

export type SourceTaskApi = {
  listSources(): Promise<SourceSubscription[]>;
  updateSource(sourceId: string, force?: boolean): Promise<SourceTaskSnapshot>;
  matchGames(instances: string[] | null, nameHints: string[], language: string | null, forceDetails: boolean): Promise<SourceTaskSnapshot>;
  rematchGame(instance: string | null, appid: number | null, nameHints: string[], language: string | null): Promise<SourceTaskSnapshot>;
  setInterval(sourceId: string, intervalHours: number): Promise<unknown>;
  projectInstances(instanceIds: string[]): Promise<ProjectInstancesResult>;
  taskStatus(): Promise<SourceTaskSnapshot[]>;
  cancelTask(taskId: string): Promise<SourceTaskSnapshot>;
};

export const sourceTaskTerminal = (state: SourceTaskState): boolean => ["completed", "cancelled", "failed"].includes(state);

export function sourceTaskPercent(task: SourceTaskSnapshot): number | null {
  const progress = task.progress;
  if (!progress || progress.total == null || progress.total <= 0 || progress.completed == null) return null;
  return Math.min(100, Math.max(0, progress.completed / progress.total * 100));
}

export function sourceTaskLabel(task: SourceTaskSnapshot): string {
  if (task.kind === "update_source") return `更新来源 ${task.key}`;
  if (task.kind === "match_games") return "匹配游戏补丁";
  return "重新匹配此游戏";
}

export function sourceTaskStatusText(task: SourceTaskSnapshot): string {
  const percent = sourceTaskPercent(task);
  const phase = task.progress?.phase ? sourcePhaseLabel(task.progress.phase) : "准备中";
  const counts = task.progress?.total != null
    ? ` ${task.progress.completed ?? 0}/${task.progress.total}` : "";
  return `${sourceTaskLabel(task)} · ${phase}${counts}${percent != null ? ` · ${Math.round(percent)}%` : ""}`;
}


/** Whether a match task targets the given instance/AppID row. */
export function sourceTaskTargets(task: SourceTaskSnapshot, instanceId: string, appid: number): boolean {
  if (task.kind === "update_source") return false;
  if (task.instances.includes(instanceId)) return true;
  return task.appid != null && task.appid === appid;
}
