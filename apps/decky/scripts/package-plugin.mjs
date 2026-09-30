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
cpSync(join(app, "backend"), join(output, "backend"), { recursive: true, filter: path => !path.includes("__pycache__") && !path.endsWith("test_bridge.py") });
cpSync(join(app, "dist"), join(output, "dist"), { recursive: true });
copyFileSync(binary, join(output, "bin", "steam-patch"));
copyFileSync(join(root, "catalog", "catalog.json"), join(output, "bin", "catalog.json"));
for (const file of ["signed-catalog.json", "trust-keys.json"]) {
  const source = join(root, "catalog", file);
  if (existsSync(source)) copyFileSync(source, join(output, "bin", file));
}
console.log(output);
