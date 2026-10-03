import { callable } from "@decky/api";
import { createTaskController, type TaskSnapshot } from "./task-controller";

export { createTaskController, taskIsTerminal, type TaskSnapshot } from "./task-controller";

const prepare = callable<[string, string, string], { task_id: string }>("start_prepare_task");
const install = callable<[string, boolean], { task_id: string }>("start_install_task");
const status = callable<[string], TaskSnapshot>("task_status");
const cancel = callable<[string], TaskSnapshot>("cancel_task");
const list = callable<[], TaskSnapshot[]>("list_tasks");

export const tasks = createTaskController({ prepare, install, status, cancel, list });
