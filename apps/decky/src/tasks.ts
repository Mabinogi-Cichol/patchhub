import { callable } from "@decky/api";
import { createTaskController, type TaskSnapshot } from "./task-controller";
import type { ExeConfirmations } from "./install";

export { createTaskController, taskIsTerminal, type TaskSnapshot } from "./task-controller";

const startPrepare = callable<[string, string, string, ExeConfirmations?], { task_id: string }>("start_prepare_task");
const prepare = (instance: string, release: string, archive: string, confirmations?: ExeConfirmations) =>
  confirmations ? startPrepare(instance, release, archive, confirmations) : startPrepare(instance, release, archive);
const install = callable<[string, boolean], { task_id: string }>("start_install_task");
const status = callable<[string], TaskSnapshot>("task_status");
const cancel = callable<[string], TaskSnapshot>("cancel_task");
const list = callable<[], TaskSnapshot[]>("list_tasks");

export const tasks = createTaskController({ prepare, install, status, cancel, list });
