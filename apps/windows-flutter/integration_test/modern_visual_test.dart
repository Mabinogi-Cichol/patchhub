import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:patchhub_windows/app.dart';
import 'package:patchhub_windows/host/app_lifecycle.dart';
import 'package:patchhub_windows/host/app_update.dart';
import 'package:patchhub_windows/host/core_client.dart';
import 'package:patchhub_windows/host/core_paths.dart';
import 'package:patchhub_windows/host/steam_artwork.dart';
import 'package:patchhub_windows/models/app_model.dart';
import 'package:patchhub_windows/models/library.dart';
import 'package:patchhub_windows/pages/library_page.dart';
import 'package:patchhub_windows/pages/app_update_panel.dart';
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

class VisualUpdate extends AppUpdate {
  VisualUpdate()
    : super(
        directory: Directory.current,
        current: '0.3.0',
        stateFile: File('unused'),
      );
  @override
  Future<void> initialize() async {}
  @override
  Future<void> check({bool manual = false}) async {
    lastCheckWasManual = manual;
    result = {'state': 'up_to_date'};
    notifyListeners();
  }
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  const dataPath = String.fromEnvironment('PATCHHUB_VISUAL_DATA');
  const outputPath = String.fromEnvironment('PATCHHUB_VISUAL_OUTPUT');
  const appid = int.fromEnvironment('PATCHHUB_VISUAL_APPID', defaultValue: 42);

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
      final location = await const MethodChannel('patchhub/desktop')
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
          const Size(1382, 1169),
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
              child: PatchHubApp(
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
          PatchHubApp.setThemeMode(
            tester.element(find.byType(LibraryPage)),
            dark ? ThemeMode.dark : ThemeMode.light,
          );
          await tester.pumpAndSettle();
          await tester.enterText(find.byType(TextField).first, '$appid');
          await tester.pumpAndSettle();
          final row = model.snapshot!.rows.firstWhere(
            (r) => r.game['appid'] == appid,
          );
          if (dataPath.isEmpty) {
            row.game['install_path'] =
                r'\\?\F:\Steam Library\steamapps\common\'
                '用于排版验收的中文游戏目录${List.filled(4, 'LongDirectoryName').join()}';
            row.game['library_path'] = r'\\?\F:\Steam Library\用于排版验收的游戏库';
          }
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

          expect(find.text('概览'), findsNothing);
          await capture('management');
          final info = find.widgetWithText(ExpansionTile, '更多游戏信息');
          await tester.ensureVisible(info);
          await tester.pumpAndSettle();
          await tester.tap(
            find.descendant(of: info, matching: find.text('更多游戏信息')),
          );
          await tester.pumpAndSettle();
          expect(find.textContaining('安装路径：'), findsOneWidget);
          final libraryPath = find.byKey(const ValueKey('game-library-path'));
          expect(libraryPath, findsOneWidget);
          await Scrollable.ensureVisible(
            tester.element(libraryPath),
            alignment: 1,
          );
          await tester.pumpAndSettle();
          expect(
            tester.getRect(libraryPath).bottom,
            lessThanOrEqualTo(
              tester
                  .getRect(find.byKey(const ValueKey('patch-floating-actions')))
                  .top,
            ),
          );
          await capture('management-game-info');
          await capture('patch');
          final technical = find.text('文件与技术详情');
          await tester.ensureVisible(technical);
          await tester.pumpAndSettle();
          await tester.tap(technical);
          await tester.pumpAndSettle();
          await tester.ensureVisible(find.textContaining('发行标识：'));
          await tester.pumpAndSettle();
          await capture('patch-expanded');
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
      final update = VisualUpdate();
      final updateModel = VisualReadModel(jsonDecode(jsonEncode(data)));
      final updateKey = GlobalKey();
      await windowManager.setSize(const Size(1180, 760));
      await tester.pumpWidget(
        PatchHubApp(
          home: RepaintBoundary(
            key: updateKey,
            child: AppUpdatePanel(
              update: update,
              lifecycle: AppLifecycle(),
              child: LibraryPage(model: updateModel),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      final before = tester.getRect(find.byType(LibraryPage));
      await tester.tap(find.text('检查更新'));
      await tester.pumpAndSettle();
      expect(find.text('当前已是最新正式版。'), findsOneWidget);
      expect(tester.getRect(find.byType(LibraryPage)), before);
      await tester.tap(find.text('关闭'));
      await tester.pumpAndSettle();
      expect(tester.getRect(find.byType(LibraryPage)), before);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox.shrink());
      updateModel.dispose();
      update.dispose();
      // Verify the caption's direct lifecycle dispatch after the screenshot
      // matrix. The observer keeps the test host alive for its final receipt.
      await windowManager.setSize(const Size(1180, 760));
      final closeModel = VisualReadModel(jsonDecode(jsonEncode(data)));
      final lifecycle = VisualCloseObserver(tasks: closeModel.tasks);
      await lifecycle.initialize();
      await tester.pumpWidget(
        PatchHubApp(
          onClose: lifecycle.onWindowClose,
          home: LibraryPage(model: closeModel),
        ),
      );
      await tester.pumpAndSettle();
      final closeWatch = Stopwatch()..start();
      await tester.tap(find.byTooltip('关闭'));
      await lifecycle.completed.future;
      closeWatch.stop();
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
          jsonEncode({
            'passed': true,
            'screenshots': metrics.length,
            'caption_close_dispatch_ms': closeWatch.elapsedMilliseconds,
          }),
        );
      }
    },
  );
}
