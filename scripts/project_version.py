"""Read the application version and synchronize tool-required version fields.

Only Cargo.toml [workspace.package].version is maintained by hand.
--sync changes project metadata, never third-party dependency resolutions.
"""
import argparse
import json
import os
from pathlib import Path
import re
import tomllib

ROOT = Path(__file__).resolve().parents[1]


def project_version(root=ROOT):
    manifest = tomllib.loads((root / 'Cargo.toml').read_text(encoding='utf-8'))
    version = manifest['workspace']['package']['version']
    identifier = r'[0-9A-Za-z-]+'
    semver = rf'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-({identifier}(?:\.{identifier})*))?(?:\+{identifier}(?:\.{identifier})*)?'
    match = re.fullmatch(semver, version) if isinstance(version, str) else None
    if not match or (match[4] and any(p.isdigit() and len(p) > 1 and p[0] == '0' for p in match[4].split('.'))):
        raise ValueError('Cargo.toml workspace.package.version must be a valid SemVer')
    tag = os.environ.get('GITHUB_REF', '')
    if tag.startswith('refs/tags/') and tag != f'refs/tags/v{version}':
        raise ValueError(f'Release Tag differs from application version: expected v{version}, got {tag}')
    return version


def flutter_version(version):
    # Retain the existing +1 build number unless SemVer already supplies metadata.
    return version if '+' in version else f'{version}+1'


def _updates(root, version):
    """Calculate every edit before writing any file; return content and field diffs."""
    updates = []
    for relative, fields in (
        ('apps/decky/package.json', [(('version',), version), (('dependencies', '@patchhub/ui'), version)]),
        ('packages/ui/package.json', [(('version',), version)]),
        ('package-lock.json', [
            (('packages', 'apps/decky', 'version'), version),
            (('packages', 'apps/decky', 'dependencies', '@patchhub/ui'), version),
            (('packages', 'packages/ui', 'version'), version),
        ]),
    ):
        data = json.loads((root / relative).read_text(encoding='utf-8'))
        differences = []
        for keys, expected in fields:
            parent = data
            for key in keys[:-1]:
                parent = parent[key]
            actual = parent[keys[-1]]
            if actual != expected:
                differences.append(f'{relative} {"/".join(keys)}: {actual!r} -> {expected!r}')
                parent[keys[-1]] = expected
        if differences:
            updates.append((relative, json.dumps(data, ensure_ascii=False, indent=2) + '\n', differences))

    relative = 'apps/windows-flutter/pubspec.yaml'
    content = (root / relative).read_text(encoding='utf-8')
    pattern = r'(?m)^(version:\s*)([^\s#]+)'
    match = re.search(pattern, content)
    if not match:
        raise ValueError(f'{relative}: missing application version')
    expected = flutter_version(version)
    if match[2] != expected:
        updated = content[:match.start(2)] + expected + content[match.end(2):]
        updates.append((relative, updated, [f'{relative} version: {match[2]!r} -> {expected!r}']))

    workspace = tomllib.loads((root / 'Cargo.toml').read_text(encoding='utf-8'))['workspace']
    members = {tomllib.loads((root / member / 'Cargo.toml').read_text(encoding='utf-8'))['package']['name']
               for member in workspace['members']}
    relative = 'Cargo.lock'
    content = (root / relative).read_text(encoding='utf-8')
    blocks = re.split(r'(?m)(?=^\[\[package\]\])', content)
    found = set()
    differences = []
    for index, block in enumerate(blocks):
        if not block.startswith('[[package]]'):
            continue
        package = tomllib.loads(block)['package'][0]
        if package['name'] not in members or 'source' in package:
            continue
        name = package['name']
        found.add(name)
        if package['version'] != version:
            blocks[index] = re.sub(r'(?m)^(version\s*=\s*)("[^"\n]*"|\x27[^\x27\n]*\x27)',
                                   lambda m: m[1] + json.dumps(version), block, count=1)
            differences.append(f'{relative} {name}: {package["version"]!r} -> {version!r}')
    if found != members:
        raise ValueError(f'Cargo.lock missing workspace packages: {", ".join(sorted(members - found))}')
    if differences:
        updates.append((relative, ''.join(blocks), differences))
    return updates


def synchronize(root=ROOT):
    version = project_version(root)
    updates = _updates(root, version)
    for relative, content, _ in updates:
        (root / relative).write_text(content, encoding='utf-8', newline='\n')
    return [relative for relative, _, _ in updates]


def check(root=ROOT):
    version = project_version(root)
    differences = [difference for _, _, fields in _updates(root, version) for difference in fields]
    if differences:
        raise ValueError('Version fields are out of sync; run python scripts/project_version.py --sync:\n'
                         + '\n'.join(differences))
    return version


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument('--sync', action='store_true', help='Update derived manifests and project lock fields')
    mode.add_argument('--check', action='store_true', help='Check all derived fields and the Release Tag without writing')
    args = parser.parse_args()
    try:
        if args.sync:
            changed = synchronize()
            print('Synchronized versions: ' + (', '.join(changed) if changed else 'already current'))
        elif args.check:
            print(f'Version fields match {check()}')
        else:
            print(project_version())
    except (ValueError, KeyError, OSError) as error:
        parser.exit(1, f'{error}\n')


if __name__ == '__main__':
    main()
