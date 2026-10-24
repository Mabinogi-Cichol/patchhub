"""Collect resolved Flutter dependencies and their actual license texts for SBOM."""
import json
from pathlib import Path
from urllib.parse import urljoin
from urllib.request import url2pathname
from urllib.parse import urlparse


def flutter_components(root, deps_path, version_path):
    deps = json.loads(Path(deps_path).read_text(encoding='utf-8-sig'))
    version = json.loads(Path(version_path).read_text(encoding='utf-8-sig'))
    config_path = root / 'apps/windows-flutter/.dart_tool/package_config.json'
    config = json.loads(config_path.read_text(encoding='utf-8'))
    locations = {p['name']: Path(url2pathname(urlparse(urljoin(config_path.as_uri(), p['rootUri'])).path))
                 for p in config['packages']}
    result = []
    for package in deps['packages']:
        if package['kind'] == 'root':
            continue
        name, release = package['name'], package['version']
        ref = f'pkg:pub/{name}@{release}'
        item = {'type': 'library', 'name': name, 'version': release, 'purl': ref, 'bom-ref': ref,
                'properties': [{'name': 'patchhub:dependency-source', 'value': package['source']}]}
        licenses = sorted(path for path in locations[name].iterdir()
                          if path.is_file() and path.name.upper().startswith(('LICENSE', 'COPYING')))
        if licenses:
            item['licenses'] = [{'license': {'name': f'{name} {path.name}',
                'text': {'contentType': 'text/plain', 'content': path.read_text(encoding='utf-8')}}}
                for path in licenses]
        else:
            item['properties'].append({'name': 'patchhub:license-review', 'value': 'required; consult bundled data/flutter_assets/NOTICES.Z'})
        result.append(item)
    for name, release in [('Flutter SDK', version['frameworkVersion']), ('Dart SDK', version['dartSdkVersion'])]:
        ref = f'flutter-runtime:{name}:{release}'
        result.append({'type': 'framework', 'name': name, 'version': release, 'bom-ref': ref,
                       'licenses': [{'license': {'name': 'BSD-3-Clause; see resources/licenses/' + name.split()[0] + '-LICENSE'}}]})
    return result
