import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:patchhub_windows/app.dart';
import 'package:patchhub_windows/host/core_client.dart';
import 'package:patchhub_windows/host/core_paths.dart';
import 'package:patchhub_windows/models/app_model.dart';
import 'package:patchhub_windows/pages/source_pages.dart';

class ImportPaths extends CorePaths {
  ImportPaths(String root, String state, this.library)
    : super(
        binary: '$root/target/debug/patchhub-core.exe',
        stateDirectory: state,
      );
  final String library;
  @override
  Map<String, Object?> get host => {
    ...super.host,
    'roots': [library],
  };
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  const root = String.fromEnvironment('PATCHHUB_IMPORT_ROOT');
  const evidence = String.fromEnvironment('PATCHHUB_IMPORT_EVIDENCE');
  testWidgets(
    'native window imports a repository source and keeps it after restart',
    (tester) async {
      final config =
          jsonDecode(
                File('../../crates/core/resources/sources.json').readAsStringSync(),
              )
              as Map<String, dynamic>;
      final rule = Map<String, dynamic>.from(
        (config['sources'] as List).firstWhere(
              (source) => (source as Map)['id'] == 'kagura-cn',
            )
            as Map,
      );
      rule
        ..['id'] = 'kagura-native'
        ..['name'] = 'Kagura native';
      final adapter = rule.remove('adapter') as String;
      final code = File(
        '../../crates/core/resources/source-scripts/'
        '${(config['bundled_scripts'] as Map)[adapter]}',
      ).readAsStringSync();
      final library = Directory('$evidence/library')..createSync(recursive: true);
      final paths = ImportPaths(root, '$evidence/state', library.path);
      final protocol = WireProtocol(
        jsonDecode(
          await rootBundle.loadString('assets/protocol-v5.schema.json'),
        ) as Map<String, dynamic>,
      );
      final client = CoreClient(paths, protocol);
      final model = AppModel(client);
      try {
        await tester.runAsync(() => model.loadSources());
        await tester.pumpWidget(
          PatchHubApp(
            home: Scaffold(body: GlobalSourcesPage(model: model)),
          ),
        );
        await tester.pumpAndSettle();
        final import = find.text('导入脚本来源');
        await tester.ensureVisible(import);
        await tester.tap(import);
        await tester.pumpAndSettle();
        Future<void> type(String key, String value) async {
          await tester.enterText(find.byKey(ValueKey('script-$key')), value);
          await tester.pump();
        }

        await type('id', rule['id'] as String);
        await type('name', rule['name'] as String);
        await tester.tap(find.text('仅发现'));
        await tester.pump();
        await type('priority', '${rule['priority']}');
        await type('pages', (rule['pages'] as List).join('\n'));
        await type('page_hosts', (rule['page_hosts'] as List).join('\n'));
        await type('code', code);
        await tester.tap(find.text('保存并导入'));
        await tester.pumpAndSettle();
        expect(find.text('Kagura native'), findsWidgets);
        final saved = await tester.runAsync(
          () => client.call('get_script_source', {
            'source_id': 'kagura-native',
          }),
        ) as Map;
        expect(saved['code'], code);
        expect(saved['rule']['discovery_only'], true);
        File('$evidence/result.json').writeAsStringSync(
          jsonEncode({
            'source': 'kagura-native',
            'sha256': saved['sha256'],
            'code_length': code.length,
          }),
        );
      } finally {
        await tester.runAsync(model.tasks.shutdown);
        model.dispose();
      }

      // A fresh client must see the same source after the app restarts.
      final reopened = CoreClient(paths, protocol);
      final sources = await reopened.call('list_sources') as List;
      expect(
        sources.map((source) => (source as Map)['id']),
        contains('kagura-native'),
      );
      final document = await reopened.call('get_script_source', {
        'source_id': 'kagura-native',
      }) as Map;
      expect(document['code'], code);
    },
    skip: root.isEmpty || evidence.isEmpty,
  );
}
