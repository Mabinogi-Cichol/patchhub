import { callable } from "@decky/api";
import { createTaskController, type TaskSnapshot } from "./task-controller";

export { createTaskController, taskIsTerminal, type TaskSnapshot } from "./task-controller";

const install = callable<[string, string | null, string | null, string | null, "install" | "reinstall", string], { task_id: string }>("start_patch_install_task");
const status = callable<[string], TaskSnapshot>("task_status");
const cancel = callable<[string], TaskSnapshot>("cancel_task");
const list = callable<[], TaskSnapshot[]>("list_tasks");

export const tasks = createTaskController({ install, status, cancel, list });
