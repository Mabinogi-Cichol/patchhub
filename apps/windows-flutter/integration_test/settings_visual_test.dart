import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:patchhub_windows/app.dart';
import 'package:patchhub_windows/host/app_lifecycle.dart';
import 'package:patchhub_windows/models/app_model.dart';
import 'package:patchhub_windows/pages/app_update_panel.dart';
import 'package:patchhub_windows/pages/library_page.dart';
import 'package:window_manager/window_manager.dart';

import '../test/app_update_panel_test.dart' show PanelUpdate;
import '../test/settings_page_test.dart' show CacheClient;

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('native settings layouts with fixture data only', (tester) async {
    await windowManager.ensureInitialized();
    await windowManager.setTitleBarStyle(
      TitleBarStyle.hidden,
      windowButtonVisibility: false,
    );
    await windowManager.setMinimumSize(const Size(850, 560));
    final output = Directory('../../.artifacts/windows-settings/visual');
    await output.create(recursive: true);
    for (final dark in [true, false]) {
      for (final size in [const Size(1180, 760), const Size(850, 560)]) {
        await windowManager.setSize(size);
        await windowManager.show();
        final model = AppModel(CacheClient());
        final update = PanelUpdate()
          ..result = {'state': 'available', 'tag': 'v0.4.0', 'notes': '测试发行说明'};
        final boundaryKey = GlobalKey();
        await tester.pumpWidget(
          RepaintBoundary(
            key: boundaryKey,
            child: PatchHubApp(
              home: Builder(
                builder: (context) {
                  return MediaQuery(
                    data: MediaQuery.of(context).copyWith(
                      textScaler: TextScaler.linear(
                        size.width == 850 ? 1.5 : 1,
                      ),
                    ),
                    child: AppUpdatePanel(
                      update: update,
                      lifecycle: AppLifecycle(),
                      child: LibraryPage(model: model),
                    ),
                  );
                },
              ),
            ),
          ),
        );
        PatchHubApp.setThemeMode(
          tester.element(find.byType(LibraryPage)),
          dark ? ThemeMode.dark : ThemeMode.light,
        );
        await tester.pumpAndSettle();
        await tester.tap(find.widgetWithText(TextButton, '设置'));
        await tester.pumpAndSettle();
        for (final (label, file) in [
          ('全局来源', 'sources'),
          ('缓存管理', 'cache'),
          ('关于', 'about'),
        ]) {
          await tester.tap(find.widgetWithText(ListTile, label));
          await tester.pumpAndSettle();
          await tester.sendKeyEvent(LogicalKeyboardKey.tab);
          await tester.pumpAndSettle();
          expect(tester.takeException(), isNull);
          expect(
            find.byKey(const ValueKey('app-update-banner')),
            findsOneWidget,
          );
          final boundary =
              boundaryKey.currentContext!.findRenderObject()!
                  as RenderRepaintBoundary;
          final image = await boundary.toImage();
          final bytes = (await image.toByteData(
            format: ui.ImageByteFormat.png,
          ))!;
          await File(
            '${output.path}/${dark ? 'dark' : 'light'}-${size.width.toInt()}-$file.png',
          ).writeAsBytes(bytes.buffer.asUint8List());
          image.dispose();
        }
        await tester.tap(find.text('返回游戏'));
        await tester.pumpAndSettle();
        await tester.tap(find.widgetWithText(TextButton, '设置'));
        await tester.pumpAndSettle();
        expect(find.text('当前版本 0.3.0'), findsOneWidget);
        expect(tester.takeException(), isNull);
        await tester.pumpWidget(const SizedBox.shrink());
        model.dispose();
        update.dispose();
      }
    }
  });
}
