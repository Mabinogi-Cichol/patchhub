import type { TaskSnapshot } from "./task-controller";

// Steam's native ProgressBar expects a percentage (0–100), not a fraction.
export function taskProgressPercent(task: TaskSnapshot | null): number | null {
  if (!task || task.total_bytes == null || !Number.isFinite(task.total_bytes) || task.total_bytes <= 0 || !Number.isFinite(task.bytes_done)) return null;
  return Math.min(100, Math.max(0, task.bytes_done / task.total_bytes * 100));
}
