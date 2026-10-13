import { progressPercent } from "@steam-patch/ui";
import type { TaskSnapshot } from "./task-controller";

export function taskProgressPercent(task: TaskSnapshot | null): number | null {
  return task ? progressPercent(task.bytes_done, task.total_bytes) : null;
}
