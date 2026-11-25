const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');

const root = path.resolve(__dirname, '../..');
const output = path.join(__dirname, '.build');
const target = path.join(__dirname, 'app.js');
const check = process.argv.includes('--check');
try {
  const args = ['--target', 'ES2020', '--module', 'ES2020', '--lib', 'ES2020,DOM', '--strict', '--skipLibCheck', '--outDir', 'apps/remote-web/.build', 'apps/remote-web/app.ts'];
  const compiler = require.resolve('typescript/bin/tsc');
  const result = spawnSync(process.execPath, [compiler, ...args], { cwd: root, stdio: 'inherit' });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exitCode = result.status || 1;
  else {
    const generated = fs.readFileSync(path.join(output, 'apps/remote-web/app.js'));
    if (check) {
      if (!fs.existsSync(target) || !generated.equals(fs.readFileSync(target))) {
        console.error('app.js is out of date; run node apps/remote-web/build.cjs');
        process.exitCode = 1;
      }
    } else fs.writeFileSync(target, generated);
  }
} finally {
  fs.rmSync(output, { recursive: true, force: true });
}
