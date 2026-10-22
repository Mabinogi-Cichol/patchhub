"""Resolved Flutter metadata includes SDK versions and actual dependency licenses."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
spec = importlib.util.spec_from_file_location('flutter_sbom', Path(__file__).with_name('flutter_sbom.py'))
collector = importlib.util.module_from_spec(spec)
spec.loader.exec_module(collector)
class FlutterInventoryTests(unittest.TestCase):
    def test_relative_package_uri_and_license_content(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            app = root / 'apps/windows-flutter'
            config = app / '.dart_tool/package_config.json'
            config.parent.mkdir(parents=True)
            package = app / 'cached-package'; package.mkdir()
            (package / 'LICENSE').write_text('Actual license text', encoding='utf-8')
            config.write_text(json.dumps({'packages': [{'name': 'sample', 'rootUri': '../cached-package'}]}))
            deps = root / 'deps.json'; deps.write_text(json.dumps({'packages': [{'name': 'sample', 'version': '2.3.4', 'source': 'hosted', 'kind': 'direct'}]}))
            version = root / 'version.json'; version.write_text(json.dumps({'frameworkVersion': '3.47.7', 'dartSdkVersion': '3.13.5'}))
            result = collector.flutter_components(root, deps, version)
            self.assertEqual([p['name'] for p in result], ['sample', 'Flutter SDK', 'Dart SDK'])
            self.assertEqual(result[0]['licenses'][0]['license']['text']['content'], 'Actual license text')
            self.assertEqual(result[0]['purl'], 'pkg:pub/sample@2.3.4')
            self.assertNotIn(str(root), json.dumps(result))
if __name__ == '__main__': unittest.main()
