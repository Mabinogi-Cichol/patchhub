import 'dart:convert';
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:window_manager/window_manager.dart';
import 'package:steam_patch_windows/app.dart';
import 'package:steam_patch_windows/host/app_lifecycle.dart';
import 'package:steam_patch_windows/host/core_client.dart';
import 'package:steam_patch_windows/host/core_paths.dart';
import 'package:steam_patch_windows/models/app_model.dart';
import 'package:steam_patch_windows/pages/install_dialog.dart';
import 'package:steam_patch_windows/pages/library_page.dart';

class AcceptancePaths extends CorePaths {
  AcceptancePaths(this.spec)
    : super(
        binary: spec['binary'],
        stateDirectory: spec['state'],
        resourceDirectory: spec['resources'],
      );
  final Map<String, dynamic> spec;
  @override
  Map<String, Object?> get host => {
    ...super.host,
    'roots': spec['roots'],
    'signed_catalog': spec['signed_catalog'],
    'trust_keys': spec['trust_keys'],
  };
}

// Observe the actual native close callback without destroying the test connection.
class ObservedLifecycle extends AppLifecycle {
  ObservedLifecycle(AppModel model)
    : super(tasks: model.tasks, onReturnToApp: model.refresh);
  bool exitObserved = false;
  @override
  Future<void> finishExit() async {
    exitObserved = true;
  }
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  const specPath = String.fromEnvironment('STEAM_PATCH_NATIVE_SPEC');
  testWidgets(
    'Windows window themes and native close cancel the real temporary ZIP task',
    (tester) async {
      if (specPath.isEmpty) {
        throw StateError(
          'Pass --dart-define=STEAM_PATCH_NATIVE_SPEC=<temporary acceptance spec.json>',
        );
      }
      final spec =
          jsonDecode(File(specPath).readAsStringSync()) as Map<String, dynamic>;
      // This acceptance target must be an isolated synthetic Steam library, not a user's library.
      if (spec['appid'] != 42) {
        throw StateError('Native acceptance uses synthetic AppID 42 only');
      }
      final temporary = Directory.systemTemp
          .resolveSymbolicLinksSync()
          .toLowerCase();
      final roots = (spec['roots'] as List).cast<String>();
      if (roots.isEmpty ||
          roots.any(
            (root) =>
                !Directory(root)
                    .resolveSymbolicLinksSync()
                    .toLowerCase()
                    .startsWith('$temporary${Platform.pathSeparator}'),
          )) {
        throw StateError(
          'Native acceptance roots must be inside the OS temporary directory',
        );
      }
      final output = Directory(spec['evidence_directory'] as String)
        ..createSync(recursive: true);
      final protocol = WireProtocol(
        jsonDecode(
          await rootBundle.loadString('assets/protocol-v5.schema.json'),
        ),
      );
      final client = CoreClient(AcceptancePaths(spec), protocol);
      for (final source in await client.call('list_sources') as List) {
        await client.call('set_source_enabled', {
          'source_id': source['id'],
          'enabled': false,
        });
      }
      final model = AppModel(client);
      await model.initialize();
      final row = model.snapshot!.rows.single;
      final lifecycle = ObservedLifecycle(model);
      await windowManager.ensureInitialized();
      await lifecycle.initialize();
      await windowManager.setTitle('Steam 黄油补丁助手 · 本地集成验收');
      await windowManager.setSize(const Size(1180, 760));
      await windowManager.show();
      final surface = GlobalKey();
      await tester.pumpWidget(
        RepaintBoundary(
          key: surface,
          child: SteamPatchApp(home: LibraryPage(model: model)),
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text(row.name));
      await tester.pumpAndSettle();
      Future<void> screenshot(String name) async {
        final boundary =
            surface.currentContext!.findRenderObject() as RenderRepaintBoundary;
        final image = await boundary.toImage(
          pixelRatio: View.of(surface.currentContext!).devicePixelRatio,
        );
        final bytes = await image.toByteData(format: ui.ImageByteFormat.png);
        await File('${output.path}/$name.png')
            .writeAsBytes(bytes!.buffer.asUint8List());
        image.dispose();
      }

      await screenshot('windows-light-overview');
      await tester.tap(find.byTooltip('外观'));
      await tester.pumpAndSettle();
      await tester.tap(
        find.ancestor(
          of: find.text('深色'),
          matching: find.byType(CheckedPopupMenuItem<ThemeMode>),
        ),
      );
      await tester.pumpAndSettle();
      expect(
        Theme.of(tester.element(find.byType(LibraryPage))).brightness,
        Brightness.dark,
      );
      await tester.tap(find.text('补丁管理'));
      await tester.pumpAndSettle();
      await windowManager.setSize(const Size(850, 560));
      await tester.pumpAndSettle();
      await screenshot('windows-dark-minimum-patch');
      expect(tester.takeException(), isNull);
      final targetFile = File('${spec['game_directory']}/patch.dat');
      final saveFile = File('${spec['game_directory']}/save.dat');
      final original = await targetFile.readAsBytes();
      final originalSave = await saveFile.readAsBytes();
      final task = model.tasks.startInstall({
        'instance_id': row.id,
        'release_id': spec['release_id'],
        'archive': spec['archive'],
        'operation': 'overwrite',
        'cleanup_policy': 'disabled',
      });
      final writing = task.events.firstWhere(
        (event) =>
            event['stage'] == 'extracting' && (event['bytes_done'] as int) > 0,
      );
      final dialog = showInstallDialog(
        tester.element(find.byType(LibraryPage)),
        model: model,
        task: task,
        game: row.name,
        patch: '临时 ZIP 原生关闭验收',
      );
      await tester.pump();
      await writing;
      await windowManager.close();
      await task.done;
      await client.waitForIdle();
      await tester.pumpAndSettle();
      expect(task.controlError, isNull);
      expect(task.error, isNull);
      expect(task.running, isFalse);
      final result = task.result as Map<String, dynamic>;
      if (result['state'] == 'failed') {
        expect(task.cancelRequested, isTrue);
        expect(lifecycle.error.toString(), contains('安装未完成'));
        expect(lifecycle.exitObserved, isFalse);
        expect(lifecycle.readyToExit, isTrue);
      } else {
        // A completed receipt remains authoritative when native close loses the race.
        expect(result['state'], 'committed');
        expect(lifecycle.exitObserved, isTrue);
      }
      await screenshot('windows-close-result');
      await File('${output.path}/native-close-result.json').writeAsString(
        const JsonEncoder.withIndent('  ').convert({
          'native_close_observed':
              lifecycle.closing ||
              lifecycle.readyToExit ||
              lifecycle.exitObserved,
          'cancel_requested': task.cancelRequested,
          'result': result,
          'lifecycle_error': lifecycle.error?.toString(),
          'exit_observed': lifecycle.exitObserved,
          'exit_code': task.exitCode,
          'diagnostics': task.diagnostics,
        }),
      );
      lifecycle.returnToApp();
      await client.waitForIdle();
      await tester.pumpAndSettle();
      expect(
        model.transactions.any(
          (record) => record['transaction_id'] == result['transaction_id'],
        ),
        isTrue,
      );
      expect(await saveFile.readAsBytes(), originalSave);
      await targetFile.writeAsBytes(original, flush: true);
      expect(await targetFile.readAsBytes(), original);
      // Dispose the result route through its actual button before ending the test.
      await tester.tap(find.text('关闭结果'));
      await tester.pumpAndSettle();
      await dialog;
      await tester.pumpWidget(const SizedBox.shrink());
      lifecycle.dispose();
      model.dispose();
      await windowManager.setPreventClose(false);
    },
  );
}
