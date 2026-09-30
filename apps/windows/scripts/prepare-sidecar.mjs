import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "../../..");
const triple = execFileSync("rustc", ["--print", "host-tuple"], { encoding: "utf8" }).trim();
if (!triple.endsWith("windows-msvc")) throw new Error("Windows MSVC host required for Windows bundle");
const source = join(root, "target", "release", "steam-patch.exe");
if (!existsSync(source)) throw new Error("Run cargo build --release -p steam-patch-cli before bundling");
const destination = join(here, "..", "src-tauri", "binaries", `steam-patch-${triple}.exe`);
mkdirSync(dirname(destination), { recursive: true });
copyFileSync(source, destination);
console.log(`Prepared ${destination}`);

for (const name of ["catalog.json", "signed-catalog.json", "trust-keys.json"]) {
  const source = join(root, "catalog", name);
  if (!existsSync(source)) throw new Error(`Missing catalog resource: ${name}`);
  copyFileSync(source, join(here, "..", "src-tauri", "resources", name));
}
