import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:steam_patch_windows/app.dart';
import 'package:steam_patch_windows/host/app_lifecycle.dart';
import 'package:steam_patch_windows/host/core_client.dart';
import 'package:steam_patch_windows/host/core_paths.dart';
import 'package:steam_patch_windows/host/steam_artwork.dart';
import 'package:steam_patch_windows/models/app_model.dart';
import 'package:steam_patch_windows/models/library.dart';
import 'package:steam_patch_windows/pages/library_page.dart';
import 'package:window_manager/window_manager.dart';

// This target only renders an existing scan snapshot. No core process is started.
class VisualReadClient extends CoreClient {
  VisualReadClient(this.data)
    : super(
        CorePaths(binary: '', stateDirectory: '', resourceDirectory: ''),
        WireProtocol(
          jsonDecode(File('assets/protocol-v5.schema.json').readAsStringSync()),
        ),
      );
  final Map<String, dynamic> data;
  @override
  Future<Object?> call(
    String method, [
    Map<String, Object?> args = const {},
  ]) async => switch (method) {
    'list_game_sources' =>
      (data['sources'] as List)
          .map((s) => <String, dynamic>{...s, 'subscribed': true})
          .toList(),
    'list_sources' => data['sources'],
    'list_transactions' => data['transactions'],
    _ => throw StateError('Visual acceptance does not permit $method'),
  };
}

class VisualReadModel extends AppModel {
  VisualReadModel(Map<String, dynamic> data) : super(VisualReadClient(data)) {
    snapshot = ScanSnapshot(data['scan']);
    sources = (data['sources'] as List).cast<Map<String, dynamic>>();
    transactions = (data['transactions'] as List).cast<Map<String, dynamic>>();
  }
  @override
  Future<void> initialize() async {}
}

class VisualCloseObserver extends AppLifecycle {
  VisualCloseObserver({super.tasks});
  bool closed = false;
  final completed = Completer<void>();
  @override
  Future<void> finishExit() async {
    closed = true;
    completed.complete();
  }
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  const dataPath = String.fromEnvironment('STEAM_PATCH_VISUAL_DATA');
  const outputPath = String.fromEnvironment('STEAM_PATCH_VISUAL_OUTPUT');
  const appid = int.fromEnvironment(
    'STEAM_PATCH_VISUAL_APPID',
    defaultValue: 42,
  );

  testWidgets(
    'native art, responsive pages and caption close use the real Windows host',
    (tester) async {
      // Desktop integration bindings use the system keyboard by default.
      // Explicitly register the input channel used by tester.enterText.
      tester.testTextInput.register();
      addTearDown(tester.testTextInput.unregister);
      await windowManager.ensureInitialized();
      await windowManager.setTitleBarStyle(
        TitleBarStyle.hidden,
        windowButtonVisibility: false,
      );
      await windowManager.setMinimumSize(const Size(850, 560));
      final Map<String, dynamic> data;
      if (dataPath.isNotEmpty) {
        data = jsonDecode(File(dataPath).readAsStringSync());
      } else {
        final cases = jsonDecode(
          File('../../packages/contracts/fixtures/v5/cases.json')
              .readAsStringSync(),
        );
        data = {
          'scan': (cases['responses'] as List).firstWhere(
            (c) => c['name'] == 'scan-success',
          )['response']['result'],
          'sources': <Map<String, dynamic>>[],
          'transactions': <Map<String, dynamic>>[],
        };
      }
      final location = await const MethodChannel('steam_patch/desktop')
          .invokeMapMethod<String, dynamic>('steamArtworkContext');
      final art = SteamArtworkStore();
      final images = await art.load(appid);
      final output = outputPath.isEmpty ? null : Directory(outputPath);
      await output?.create(recursive: true);
      if (output != null) {
        await File('${output.path}/artwork-context.json').writeAsString(
          jsonEncode({
            'steamRootFound': location?['root'] != null,
            'hero': images.hero,
            'cover': images.cover,
            'thumbnail': images.thumbnail,
            'diagnostics': images.diagnostics,
          }),
        );
      }
      if (dataPath.isNotEmpty) {
        expect(images.hero, isNotNull);
        expect(images.cover, isNotNull);
      }
      final metrics = <Map<String, Object?>>[];
      for (final dark in [true, false]) {
        for (final size in [
          const Size(1440, 900),
          const Size(1180, 760),
          const Size(850, 560),
        ]) {
          debugPrint(
            'Visual matrix: dark=$dark, requested=${size.width}x${size.height}',
          );
          await windowManager.setSize(size);
          final model = VisualReadModel(jsonDecode(jsonEncode(data)));
          final key = GlobalKey();
          final scale = size.width == 850 ? 1.5 : 1.0;
          await tester.pumpWidget(
            RepaintBoundary(
              key: key,
              child: SteamPatchApp(
                home: Builder(
                  builder: (context) => MediaQuery(
                    data: MediaQuery.of(context)
                        .copyWith(textScaler: TextScaler.linear(scale)),
                    child: LibraryPage(model: model, artwork: art),
                  ),
                ),
              ),
            ),
          );
          SteamPatchApp.setThemeMode(
            tester.element(find.byType(LibraryPage)),
            dark ? ThemeMode.dark : ThemeMode.light,
          );
          await tester.pumpAndSettle();
          await tester.enterText(find.byType(TextField).first, '$appid');
          await tester.pumpAndSettle();
          final row = model.snapshot!.rows.firstWhere(
            (r) => r.game['appid'] == appid,
          );
          if (find.text(row.name).evaluate().isEmpty && output != null) {
            debugPrint(
              'Visible text: ${tester.widgetList<Text>(find.byType(Text)).map((t) => t.data).toList()}',
            );
            final boundary =
                key.currentContext!.findRenderObject() as RenderRepaintBoundary;
            final image = await boundary.toImage();
            final bytes = await image.toByteData(
              format: ui.ImageByteFormat.png,
            );
            await File('${output.path}/missing-row.png')
                .writeAsBytes(bytes!.buffer.asUint8List());
            image.dispose();
          }
          await tester.tap(find.text(row.name));
          await tester.pumpAndSettle();
          await tester.enterText(find.byType(TextField).first, '');
          await tester.pumpAndSettle();
          Future<void> capture(String page) async {
            final context = tester.element(find.byType(LibraryPage));
            for (final widget in tester.widgetList<Image>(find.byType(Image))) {
              await precacheImage(widget.image, context);
            }
            await tester.pumpAndSettle();
            expect(tester.takeException(), isNull);
            final boundary =
                key.currentContext!.findRenderObject() as RenderRepaintBoundary;
            metrics.add({
              'theme': dark ? 'dark' : 'light',
              'width': boundary.size.width,
              'height': boundary.size.height,
              'textScale': scale,
              'page': page,
            });
            if (output != null) {
              final image = await boundary.toImage();
              final bytes = await image.toByteData(
                format: ui.ImageByteFormat.png,
              );
              await File(
                '${output.path}/${dark ? 'dark' : 'light'}-${size.width.toInt()}-$page.png',
              ).writeAsBytes(bytes!.buffer.asUint8List());
              image.dispose();
            }
          }

          await capture('overview');
          await tester.tap(find.text('补丁管理'));
          await tester.pumpAndSettle();
          await capture('patch');
          if (size.width == 1180) {
            for (final page in {
              '操作记录': 'records',
              '游戏订阅': 'subscriptions',
              '全局来源': 'sources',
            }.entries) {
              await tester.tap(find.text(page.key));
              await tester.pumpAndSettle();
              await capture(page.value);
            }
            await tester.tap(find.text('返回游戏'));
            await tester.pumpAndSettle();
            await tester.tap(find.byTooltip('最大化或还原'));
            await tester.pumpAndSettle();
            expect(await windowManager.isMaximized(), isTrue);
            await tester.tap(find.byTooltip('最大化或还原'));
            await tester.pumpAndSettle();
            expect(await windowManager.isMaximized(), isFalse);
          }
          await tester.pumpWidget(const SizedBox.shrink());
          await tester.pumpAndSettle();
          model.dispose();
        }
      }
      // A real WM_CLOSE changes native window activation. Verify it only after
      // the screenshot matrix, rather than reusing that window for another size.
      await windowManager.setSize(const Size(1180, 760));
      final closeModel = VisualReadModel(jsonDecode(jsonEncode(data)));
      await tester.pumpWidget(
        SteamPatchApp(home: LibraryPage(model: closeModel)),
      );
      await tester.pumpAndSettle();
      final lifecycle = VisualCloseObserver(tasks: closeModel.tasks);
      await lifecycle.initialize();
      await tester.tap(find.byTooltip('关闭'));
      await lifecycle.completed.future;
      await tester.pumpAndSettle();
      expect(lifecycle.closed, isTrue);
      lifecycle.dispose();
      await windowManager.setPreventClose(false);
      await tester.pumpWidget(const SizedBox.shrink());
      closeModel.dispose();
      if (output != null) {
        await File('${output.path}/metrics.json')
            .writeAsString(jsonEncode(metrics));
        await File('${output.path}/result.json').writeAsString(
          jsonEncode({'passed': true, 'screenshots': metrics.length}),
        );
      }
    },
  );
}
