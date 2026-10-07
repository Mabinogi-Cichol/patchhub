import { cpSync, copyFileSync, existsSync, mkdirSync, rmSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const app = resolve(here, "..");
const root = resolve(app, "../..");
const binary = join(root, "target", "linux", "release", "steam-patch");
if (!existsSync(binary) || !statSync(binary).isFile()) throw new Error("Build the Linux x86_64 CLI first: cargo build --release --target x86_64-unknown-linux-gnu -p steam-patch-cli");
const output = join(app, "plugin-output");
rmSync(output, { recursive: true, force: true });
mkdirSync(join(output, "bin"), { recursive: true });
for (const file of ["main.py", "plugin.json", "package.json", "README.md"]) copyFileSync(join(app, file), join(output, file));
copyFileSync(join(root, "LICENSE"), join(output, "LICENSE"));
cpSync(join(app, "backend"), join(output, "backend"), { recursive: true, filter: path => !path.includes("__pycache__") && !/[\\/]fixtures(?:[\\/]|$)/.test(path) && !/[\\/]test_[^\\/]*\.py$/.test(path) });
cpSync(join(app, "dist"), join(output, "dist"), { recursive: true });
mkdirSync(join(output, "backend", "exe_worker"), { recursive: true });
for (const file of ["sf95_audit.py", "sf95_worker.py", "sf9_metadata.py", "sf9_graph.py", "sf9_lua.py", "sf9_behavior.py"]) copyFileSync(join(root, "scripts", file), join(output, "backend", "exe_worker", file));
copyFileSync(join(root, "scripts", "licenses", "sfextract-MIT.txt"), join(output, "backend", "exe_worker", "sfextract-MIT.txt"));
copyFileSync(binary, join(output, "bin", "steam-patch"));
copyFileSync(join(root, "catalog", "catalog.json"), join(output, "bin", "catalog.json"));
for (const file of ["signed-catalog.json", "trust-keys.json"]) {
  const source = join(root, "catalog", file);
  if (existsSync(source)) copyFileSync(source, join(output, "bin", file));
}
console.log(output);
