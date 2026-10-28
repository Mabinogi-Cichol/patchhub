import { readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';
const require = createRequire(import.meta.url);
const ts = require('typescript');
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const source = readFileSync(resolve(root, 'packages/ui/src/index.ts'), 'utf8');
const module = { exports: {} };
new Function('exports', 'module', ts.transpileModule(source, {compilerOptions: {module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022}}).outputText)(module.exports, module);
const ui = module.exports;
const corpus = JSON.parse(readFileSync(resolve(root, 'packages/contracts/fixtures/v5/cases.json'), 'utf8'));
const original = corpus.responses.find(c => c.name === 'scan-success').response.result.installs[0];
const candidate = corpus.events.find(c => c.name === 'match_update').event.candidates[0];
const clone = value => structuredClone(value);
const scenarios = [];
const add = (name, row) => scenarios.push({name, row,
  entries: ui.displayPatches(row).map(entry => ({kind: entry.kind, key: entry.key,
    title: entry.kind === 'installable' ? entry.patch.name : entry.candidate.patch_title,
    choices: entry.kind === 'installable' ? entry.choices.map(patch => patch.selection_id ?? patch.release_id) : []})),
  matched: ui.hasMatchedPatch(row), attention: ui.needsAttention(row), summary: ui.installSummary(row),
  filters: Object.fromEntries(['matched','all','installed','attention'].map(filter => [filter, ui.filterInstalls([row], filter, '').map(item => item.game.instance_id)]))});
add('signed-single', clone(original));
let row = clone(original); row.patches[0].detection.status = 'installed_external'; add('external-install', row);
row = clone(original); row.patches = []; row.discovery.candidates = [{...candidate, match_basis: 'title'}]; add('unconfirmed-title', row);
row = clone(original); row.patches = []; row.discovery.candidates = [{...candidate, match_basis: 'appid'}]; add('strong-source-match', row);
row = clone(original); row.patches = []; row.discovery.status = 'no_match'; add('no-match', row);
row = clone(original); row.patches = []; row.discovery.status = 'publisher_match'; row.discovery.candidates = [{...candidate, source_id: 'hgamer'}]; add('suppress-fallback-next-to-publisher', row);
row = clone(original); const base = {...row.patches[0], candidate_id: candidate.candidate_id, source_id: candidate.source_id,
  selection_id: 'automatic', download_url: 'https://example.com/patch.exe'};
row.discovery.candidates = [{...candidate, patch_title: 'Grouped patch'}];
row.patches = [base, {...clone(base), selection_id: 'manual-same-file', compatibility: 'manual_selection_required'},
  {...clone(base), selection_id: 'manual-mirror', compatibility: 'manual_selection_required', download_url: 'https://mirror.example.com/patch.exe'}];
add('manual-duplicate-and-mirror', row);
row = clone(original); row.discovery.candidates = ['dlc','unknown','base'].map((variant, index) => ({...candidate, candidate_id: `candidate-${index}`, variant}));
row.patches = row.discovery.candidates.map(c => ({...clone(original.patches[0]), selection_id: `selection-${c.variant}`, candidate_id: c.candidate_id}));
add('variant-order', row);
row = clone(original); row.patches = [];
row.discovery.candidates = [{...candidate, candidate_id: 'english', languages: ['en']}, {...candidate, candidate_id: 'chinese', languages: ['zh-CN']},
  {...candidate, candidate_id: 'fallback', source_id: 'hgamer', languages: ['zh-CN']}];
add('language-variants', row);
const output = `${JSON.stringify(scenarios, null, 2)}\n`;
const destination = resolve(root, 'apps/windows-flutter/test/fixtures/presentation.json');
if (process.argv.includes('--check')) {
  if (readFileSync(destination, 'utf8') !== output) throw new Error('Flutter presentation fixture differs from shared TypeScript behavior');
  console.log('Flutter presentation fixture matches shared TypeScript behavior');
} else {
  writeFileSync(destination, output);
  console.log(`Wrote ${scenarios.length} cross-language presentation cases`);
}
