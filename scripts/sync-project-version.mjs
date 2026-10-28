import { spawnSync } from "node:child_process";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const script = resolve(root, "scripts/project_version.py");

export function syncProjectVersion() {
  let python = process.platform === "win32" ? "python" : "python3";
  const options = { cwd: root, stdio: "inherit" };
  let result = spawnSync(python, [script, "--sync"], options);
  // The Windows BAT entrypoints also support installations exposing only py.
  if (process.platform === "win32" && result.error?.code === "ENOENT") {
    python = "py";
    result = spawnSync(python, ["-3", script, "--sync"], options);
  }
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${python} ${script} --sync failed with exit code ${result.status}`);
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  syncProjectVersion();
}
