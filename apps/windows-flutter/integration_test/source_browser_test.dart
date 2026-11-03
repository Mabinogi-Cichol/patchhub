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

class SourcePaths extends CorePaths {
  SourcePaths(String root, String state, this.library)
    : super(
        binary: '$root/target/release/patchhub-core.exe',
        stateDirectory: state,
      );
  final String library;
  @override
  Map<String, Object?> get host => {
    ...super.host,
    'roots': [library],
  };
}

class ObservedClient extends CoreClient {
  ObservedClient(super.paths, super.protocol);
  int browserCalls = 0;
  @override
  Future<List<Map<String, dynamic>>> collectSourcePages(
    String id,
    Future<void> cancelled,
    void Function(String) progress,
  ) {
    browserCalls++;
    return super.collectSourcePages(id, cancelled, progress);
  }
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  const root = String.fromEnvironment('PATCHHUB_BROWSER_ROOT');
  const evidence = String.fromEnvironment('PATCHHUB_BROWSER_EVIDENCE');
  testWidgets(
    'ordinary update and scheduler use the real background browser without manual steps',
    (tester) async {
      final library = Directory('$evidence/library');
      await Directory('${library.path}/steamapps/common/Sacrifice')
          .create(recursive: true);
      await File('${library.path}/steamapps/appmanifest_3137110.acf')
          .writeAsString(
            '"AppState" { "appid" "3137110" "name" "SACRIFICE VILLAINS" "installdir" "Sacrifice" "StateFlags" "4" }',
          );
      final paths = SourcePaths(root, '$evidence/state', library.path);
      final client = ObservedClient(
        paths,
        WireProtocol(
          jsonDecode(
            await rootBundle.loadString('assets/protocol-v5.schema.json'),
          ) as Map<String, dynamic>,
        ),
      );
      final model = AppModel(client);
      try {
        await tester.runAsync(model.loadSources);
        await tester.pumpWidget(
          PatchHubApp(
            home: Scaffold(body: GlobalSourcesPage(model: model)),
          ),
        );
        await tester.pumpAndSettle();
        await tester.tap(find.text('Shiravune'));
        await tester.pumpAndSettle();
        final button = find.text('更新来源索引').first;
        await tester.ensureVisible(button);
        await tester.tap(button);
        final ordinary = model.tasks.sources['update_source:patchmoe']!;
        await tester.runAsync(() async {
          await ordinary.done;
          await client.waitForIdle();
        });
        expect(
          ordinary.error,
          isNull,
          reason: '${ordinary.error}\n${ordinary.diagnostics}',
        );
        expect(ordinary.result['complete'], true);
        expect(find.text('在网页验证并更新'), findsNothing);

        await tester.runAsync(() async {
          await client.call('set_source_update_interval', {
            'source_id': 'patchmoe',
            'interval_hours': 1,
          });
          final statusFile = File(
            '$evidence/state/sources/rust-v1/status/patchmoe.json',
          );
          final status = jsonDecode(await statusFile.readAsString()) as Map;
          // Controlled overdue fixture; the task and website run against real time.
          status['checked_at'] =
              DateTime.now().millisecondsSinceEpoch ~/ 1000 - 7200;
          await statusFile.writeAsString(jsonEncode(status));
          await model.schedulerTick();
          final scheduled = model.tasks.sources['update_source:patchmoe']!;
          expect(identical(ordinary, scheduled), false);
          await scheduled.done;
          expect(
            scheduled.error,
            isNull,
            reason: '${scheduled.error}\n${scheduled.diagnostics}',
          );
          expect(scheduled.result['complete'], true);
          final index = jsonDecode(
            await File('$evidence/state/sources/rust-v1/index/patchmoe.json')
                .readAsString(),
          ) as Map;
          expect(index['complete'], true);
          final entry = (index['entries'] as List).cast<Map>().singleWhere(
            (entry) => entry['title'] == 'SACRIFICE VILLAINS：牺牲恶棍',
          );
          expect(entry['aliases'], contains('SACRIFICE VILLAINS'));
          final match = jsonDecode(
            await File('$evidence/state/sources/rust-v1/matches/3137110.json')
                .readAsString(),
          ) as Map;
          final candidate =
              (match['sources']['patchmoe']['candidates'] as List).single;
          expect(
            candidate['mirrors'][0]['url'],
            'https://download.patch.moe/SACRIFICE_VILLAINS_Patch.zip',
          );
          expect(candidate['match_basis'], 'title');
          expect(match['status'], 'incomplete');
          await File('$evidence/result.json').writeAsString(
            jsonEncode({
              'ordinary_complete': ordinary.result['complete'],
              'scheduled_complete': scheduled.result['complete'],
              'browser_calls': client.browserCalls,
              'entries': (index['entries'] as List).length,
              'match_status': match['status'],
              'candidate_title': candidate['patch_title'],
            }),
          );
        });
      } finally {
        await tester.runAsync(model.tasks.shutdown);
        model.dispose();
      }
    },
    skip: root.isEmpty || evidence.isEmpty,
  );
}
