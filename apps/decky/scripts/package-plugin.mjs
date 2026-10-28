import { cpSync, copyFileSync, existsSync, mkdirSync, rmSync, statSync, readFileSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { syncProjectVersion } from "../../../scripts/sync-project-version.mjs";

syncProjectVersion();

const here = dirname(fileURLToPath(import.meta.url));
const app = resolve(here, "..");
const root = resolve(app, "../..");
const binaries = ["patchhub-core", "patchhub-worker"];
for (const name of binaries) {
  const binary = join(root, "target/linux/release", name);
  if (!existsSync(binary) || !statSync(binary).isFile() || !readFileSync(binary).subarray(0, 4).equals(Buffer.from([0x7f, 0x45, 0x4c, 0x46])))
    throw new Error(`Build the Linux ELF ${name} first: scripts/build-linux.sh`);
}
const output = join(app, "plugin-output");
rmSync(output, { recursive: true, force: true });
mkdirSync(join(output, "bin"), { recursive: true });
mkdirSync(join(output, "backend"), { recursive: true });
for (const file of ["main.py", "plugin.json", "package.json", "README.md"]) copyFileSync(join(app, file), join(output, file));
copyFileSync(join(root, "LICENSE"), join(output, "LICENSE"));
// An explicit production allowlist prevents developer Python oracles from
// becoming a second installation engine in the deployed plugin.
for (const file of ["bridge.py", "source_service.py", "tasks.py", "plugin_settings.py", "state_host.py", "protocol_validation.py", "app_update.py"])
  copyFileSync(join(app, "backend", file), join(output, "backend", file));
copyFileSync(join(root, 'scripts/install-decky.py'), join(output, 'backend/install-decky.py'));
copyFileSync(join(root, "packages/contracts/generated/protocol.py"), join(output, "backend/protocol.py"));
copyFileSync(join(root, "packages/contracts/generated/v5.schema.json"), join(output, "backend/v5.schema.json"));
cpSync(join(app, "dist"), join(output, "dist"), { recursive: true });
mkdirSync(join(output, "licenses"), { recursive: true });
copyFileSync(join(root, "scripts/licenses/sfextract-MIT.txt"), join(output, "licenses/sfextract-MIT.txt"));
const runtime = [];
for (const name of binaries) {
  const binary = join(root, "target/linux/release", name);
  const bytes = readFileSync(binary);
  copyFileSync(binary, join(output, "bin", name));
  runtime.push({ path: `bin/${name}`, size: bytes.length, sha256: createHash("sha256").update(bytes).digest("hex") });
}
writeFileSync(join(output, "bin/runtime.json"), JSON.stringify({ protocol_version: 5, worker_protocol: 1, files: runtime }, null, 2) + "\n");
copyFileSync(join(root, "catalog/catalog.json"), join(output, "bin/catalog.json"));
for (const file of ["signed-catalog.json", "trust-keys.json"]) {
  const source = join(root, "catalog", file);
  if (existsSync(source)) copyFileSync(source, join(output, "bin", file));
}
console.log(output);
