import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/host/core_client.dart';
import 'package:patchhub_windows/host/core_paths.dart';
import 'package:patchhub_windows/host/core_task.dart';
import 'package:patchhub_windows/pages/script_source_editor.dart';

/// Restores the five bundled sources through the ordinary import form against a
/// build whose `sources.json` has no bundled rules or scripts.
class EmptySourcePaths extends CorePaths {
  EmptySourcePaths(String binary, String state, this.library)
    : super(binary: binary, stateDirectory: state);
  final String library;
  @override
  Map<String, Object?> get host => {...super.host, 'roots': [library]};
}

class EmptyBuiltinFixture {
  EmptyBuiltinFixture() {
    final config =
        jsonDecode(
              File(
                '../../crates/core/resources/sources.json',
              ).readAsStringSync(),
            )
            as Map<String, dynamic>;
    final scripts = config['bundled_scripts'] as Map<String, dynamic>;
    for (final raw in config['sources'] as List) {
      final rule = Map<String, dynamic>.from(raw as Map);
      final adapter = rule['adapter'] as String;
      rule.remove('adapter');
      final code = File(
        '../../crates/core/resources/source-scripts/${scripts[adapter]}',
      ).readAsStringSync();
      entries[rule['id'] as String] = (rule: rule, code: code);
    }
  }

  final entries = <String, ({Map<String, dynamic> rule, String code})>{};
}

String? get emptyCore => Platform.environment['PATCHHUB_EMPTY_CORE'];

bool get emptyCoreReady {
  final path = emptyCore;
  return path != null && File(path).existsSync();
}

Directory prepareLibrary(String path) {
  final library = Directory('$path/library');
  for (final (appid, name, dir) in [
    (1227890, 'Summer Memories', 'Summer Memories'),
    (2303800, 'My Life as a Cult Leader', 'My Life as a Cult Leader'),
  ]) {
    Directory(
      '${library.path}/steamapps/common/$dir',
    ).createSync(recursive: true);
    File('${library.path}/steamapps/appmanifest_$appid.acf').writeAsStringSync(
      '"AppState" { "appid" "$appid" "name" "$name" "installdir" "$dir" "StateFlags" "4" }',
    );
  }
  return library;
}

WireProtocol protocol() => WireProtocol(
  jsonDecode(File('assets/protocol-v5.schema.json').readAsStringSync())
      as Map<String, dynamic>,
);

void main() {
  testWidgets(
    'empty build imports every bundled source through the import form',
    (tester) async {
      final temp = Directory.systemTemp.createTempSync('patchhub-empty-');
      addTearDown(() => temp.deleteSync(recursive: true));
      final library = prepareLibrary(temp.path);
      final client = CoreClient(
        EmptySourcePaths(
          File(emptyCore!).absolute.path,
          '${temp.path}/state',
          library.path,
        ),
        protocol(),
      );
      final fixture = EmptyBuiltinFixture();

      await tester.runAsync(() async {
        expect(await client.call('list_sources'), isEmpty);
      });

      for (final entry in fixture.entries.entries) {
        final id = entry.key;
        final rule = entry.value.rule;
        Map<String, Object?>? submitted;
        await tester.pumpWidget(
          MaterialApp(
            home: Scaffold(
              body: Builder(
                builder: (context) => TextButton(
                  onPressed: () => showDialog<void>(
                    context: context,
                    builder: (_) =>
                        ScriptSourceEditor(onSave: (params) async => submitted = params),
                  ),
                  child: const Text('open-editor'),
                ),
              ),
            ),
          ),
        );
        await tester.tap(find.text('open-editor'));
        await tester.pumpAndSettle();
        Future<void> type(String key, String value) async {
          await tester.enterText(find.byKey(ValueKey('script-$key')), value);
          await tester.pump();
        }

        await type('id', rule['id'] as String);
        await type('name', rule['name'] as String);
        if (rule['discovery_only'] == true) {
          await tester.tap(find.text('仅发现'));
          await tester.pump();
        }
        if (rule['role'] == 'fallback_indexer') {
          await tester.tap(find.text('发行商'));
          await tester.pumpAndSettle();
          await tester.tap(find.text('补查来源').last);
          await tester.pumpAndSettle();
        }
        final priority = rule['priority'];
        if (priority != null && priority != 100) {
          await type('priority', '$priority');
        }
        await type('pages', (rule['pages'] as List).join('\n'));
        await type('page_hosts', (rule['page_hosts'] as List).join('\n'));
        await type(
          'download_hosts',
          (rule['download_hosts'] as List?)?.join('\n') ?? '',
        );
        await type('code', entry.value.code);

        final save = find.byType(FilledButton);
        await tester.tap(save);
        await tester.pumpAndSettle();
        expect(submitted, isNotNull, reason: id);
        final saved = await tester.runAsync(() async {
          await client.call('import_script_source', submitted!);
          return client.call('get_script_source', {'source_id': id});
        }) as Map;
        expect(saved['code'], entry.value.code, reason: id);
        for (final field in rule.keys) {
          expect(
            saved['rule'][field],
            rule[field],
            reason: '$id.$field',
          );
        }
      }

      final listed = await tester.runAsync(() => client.call('list_sources'))
          as List;
      expect(
        listed.map((source) => (source as Map)['id']).toSet(),
        fixture.entries.keys.toSet(),
      );
      expect(
        listed.every((source) => (source as Map)['origin'] == 'added'),
        true,
      );
    },
    skip: !emptyCoreReady,
  );

  test(
    'restored sources refresh against the real sites and dual ids agree',
    () async {
      final temp = Directory.systemTemp.createTempSync('patchhub-empty-live-');
      addTearDown(() => temp.delete(recursive: true));
      final library = prepareLibrary(temp.path);
      final client = CoreClient(
        EmptySourcePaths(
          File(emptyCore!).absolute.path,
          '${temp.path}/state',
          library.path,
        ),
        protocol(),
      );
      final fixture = EmptyBuiltinFixture();

      Future<dynamic> task(String method, [Map<String, Object?> args = const {}]) async {
        final run = CoreTask.start(client, method, args);
        await run.done;
        if (run.error != null) {
          throw StateError('$method failed: ${run.error!.code} ${run.error!.detail}');
        }
        return run.result;
      }

      expect(await client.call('list_sources'), isEmpty);
      for (final entry in fixture.entries.entries) {
        await client.call('import_script_source', {
          'rule': entry.value.rule,
          'api_version': 1,
          'code': entry.value.code,
        });
      }

      for (final id in ['kagura-cn', 'kagura-en', 'yozakura']) {
        final result = await task('update_source', {
          'source_id': id,
          'force': true,
        }) as Map;
        expect(result['complete'], true, reason: '$id ${result['status']}');
        expect((result['status'] as Map)['entry_count'], greaterThan(0));
      }

      final original = (await task('update_source', {
        'source_id': 'kagura-cn',
        'force': true,
      }) as Map)['status'] as Map;
      await client.call('remove_source', {'source_id': 'kagura-cn'});
      await client.call('import_script_source', {
        'rule': {...fixture.entries['kagura-cn']!.rule, 'id': 'kagura-cn-copy'},
        'api_version': 1,
        'code': fixture.entries['kagura-cn']!.code,
      });
      final copy = (await task('update_source', {
        'source_id': 'kagura-cn-copy',
        'force': true,
      }) as Map)['status'] as Map;
      expect(copy['entry_count'], original['entry_count']);
      expect(copy['code'], original['code']);

      final scan = await client.call('scan') as Map;
      expect((scan['installs'] as List), isNotEmpty);
      final matched = await task('match_games') as Map;
      expect(matched.keys, isNotEmpty);
    },
    skip: !emptyCoreReady,
    timeout: const Timeout(Duration(minutes: 8)),
  );
}
