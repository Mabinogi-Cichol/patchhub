import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "../../..");
const triple = execFileSync("rustc", ["--print", "host-tuple"], { encoding: "utf8" }).trim();
if (!triple.endsWith("windows-msvc")) throw new Error("Windows MSVC host required for Windows bundle");
for (const { name, build } of [
  { name: "steam-patch", build: "cargo build --release -p steam-patch-cli" },
  { name: "sf-worker", build: "cargo build --release -p steam-patch-installer-worker --bin sf-worker" },
]) {
  const source = join(root, "target", "release", `${name}.exe`);
  if (!existsSync(source)) throw new Error(`Missing ${name}.exe; run ${build} before bundling`);
  const destination = join(here, "..", "src-tauri", "binaries", `${name}-${triple}.exe`);
  mkdirSync(dirname(destination), { recursive: true });
  copyFileSync(source, destination);
  console.log(`Prepared ${destination}`);
}

for (const name of ["catalog.json", "signed-catalog.json", "trust-keys.json"]) {
  const source = join(root, "catalog", name);
  if (!existsSync(source)) throw new Error(`Missing catalog resource: ${name}`);
  copyFileSync(source, join(here, "..", "src-tauri", "resources", name));
}

const resources = join(here, "..", "src-tauri", "resources");
mkdirSync(join(resources, "licenses"), { recursive: true });
copyFileSync(join(root, "LICENSE"), join(resources, "LICENSE"));
copyFileSync(join(root, "scripts", "licenses", "sfextract-MIT.txt"), join(resources, "licenses", "sfextract-MIT.txt"));
