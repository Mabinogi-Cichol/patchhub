import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/host/core_client.dart';
import 'package:patchhub_windows/host/core_paths.dart';
import 'package:patchhub_windows/host/core_task.dart';
import 'package:patchhub_windows/pages/script_source_editor.dart';

/// Restores the five bundled sources by pasting only their Lua code into the
/// ordinary import dialog against a build whose `sources.json` has no builtins.
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
      final entry = Map<String, dynamic>.from(raw as Map);
      final adapter = entry['adapter'] as String;
      final code = File(
        '../../crates/core/resources/source-scripts/${scripts[adapter]}',
      ).readAsStringSync();
      entries[entry['id'] as String] = code;
    }
  }

  final entries = <String, String>{};
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
    'empty build imports every bundled source with zero form fields',
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

      final importedIds = <String>{};
      for (final entry in fixture.entries.entries) {
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
        await tester.enterText(
          find.byKey(const ValueKey('script-code')),
          entry.value,
        );
        await tester.pump();

        await tester.tap(find.byType(FilledButton));
        await tester.pumpAndSettle();
        expect(submitted, isNotNull, reason: entry.key);
        expect(submitted!.containsKey('rule'), isFalse);
        expect(submitted!.containsKey('source_id'), isFalse);

        final saved = await tester.runAsync(() async {
          final added = await client.call('import_script_source', submitted!)
              as Map;
          final assignedId = (added['rule'] as Map)['id'] as String;
          importedIds.add(assignedId);
          return client.call('get_script_source', {'source_id': assignedId});
        }) as Map;
        expect(saved['code'], entry.value, reason: entry.key);
        expect(saved['rule']['adapter'], 'lua', reason: entry.key);
      }

      final listed = await tester.runAsync(() => client.call('list_sources'))
          as List;
      expect(
        listed.map((source) => (source as Map)['id']).toSet(),
        importedIds,
      );
      expect(listed.length, 5);
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

      Future<dynamic> task(String method, [Map<String, Object?> args = const {}]) async {
        final run = CoreTask.start(client, method, args);
        await run.done;
        if (run.error != null) {
          throw StateError('$method failed: ${run.error!.code} ${run.error!.detail}');
        }
        return run.result;
      }

      expect(await client.call('list_sources'), isEmpty);
      final idsByKey = <String, String>{};
      for (final entry in fixture.entries.entries) {
        final added = await client.call('import_script_source', {
          'api_version': 1,
          'code': entry.value,
        }) as Map;
        idsByKey[entry.key] = (added['rule'] as Map)['id'] as String;
      }

      for (final key in ['kagura-cn', 'kagura-en', 'yozakura']) {
        final id = idsByKey[key]!;
        final result = await task('update_source', {
          'source_id': id,
          'force': true,
        }) as Map;
        expect(result['complete'], true, reason: '$id ${result['status']}');
        expect((result['status'] as Map)['entry_count'], greaterThan(0));
      }

      final kaguraCnId = idsByKey['kagura-cn']!;
      final original = (await task('update_source', {
        'source_id': kaguraCnId,
        'force': true,
      }) as Map)['status'] as Map;
      await client.call('remove_source', {'source_id': kaguraCnId});
      final reimported = await client.call('import_script_source', {
        'api_version': 1,
        'code': fixture.entries['kagura-cn']!,
      }) as Map;
      final copyId = (reimported['rule'] as Map)['id'] as String;
      final copy = (await task('update_source', {
        'source_id': copyId,
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
