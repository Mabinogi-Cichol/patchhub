import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/app.dart';
import 'package:patchhub_windows/host/app_lifecycle.dart';
import 'package:patchhub_windows/host/core_client.dart';
import 'package:patchhub_windows/models/app_model.dart';
import 'package:patchhub_windows/pages/app_update_panel.dart';
import 'package:patchhub_windows/pages/cache_page.dart';
import 'package:patchhub_windows/pages/library_page.dart';

import 'app_update_panel_test.dart' show PanelUpdate;
import 'library_page_test.dart' show FixtureClient;

class CacheClient extends FixtureClient {
  Map<String, dynamic> inventory = {
    'total_bytes': 2048,
    'entries': 2,
    'diagnostics': [],
  };
  Object? statError, cleanError;
  Completer<Object?>? pending;
  int cleans = 0, stats = 0;
  Map<String, dynamic> cleanupResult = {
    'status': 'completed',
    'performed': true,
    'removed_bytes': 1024,
    'failures': [],
  };
  @override
  Future<Object?> call(
    String method, [
    Map<String, Object?> args = const {},
  ]) async {
    if (method == 'cache_status') {
      stats++;
      if (statError != null) throw statError!;
      return inventory;
    }
    if (method == 'cleanup_cache') {
      cleans++;
      if (cleanError != null) throw cleanError!;
      return pending == null ? cleanupResult : await pending!.future;
    }
    return super.call(method, args);
  }
}

void main() {
  Future<void> confirm(WidgetTester tester) async {
    await tester.ensureVisible(find.widgetWithText(FilledButton, '清理缓存'));
    await tester.tap(find.widgetWithText(FilledButton, '清理缓存'));
    await tester.pumpAndSettle();
    await tester.tap(
      find.descendant(
        of: find.byType(AlertDialog),
        matching: find.widgetWithText(FilledButton, '清理缓存'),
      ),
    );
    await tester.pumpAndSettle();
  }

  testWidgets(
    'cleanup survives navigation and preserves library selection, filter and search',
    (tester) async {
      final client = CacheClient()..pending = Completer<Object?>();
      final model = AppModel(client);
      final update = PanelUpdate();
      await tester.pumpWidget(
        PatchHubApp(
          home: AppUpdatePanel(
            update: update,
            lifecycle: AppLifecycle(),
            child: LibraryPage(model: model),
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.text('检查更新'), findsNothing);
      expect(find.byKey(const ValueKey('app-update-banner')), findsNothing);
      expect(client.stats, 0);
      await tester.tap(find.text('查看全部游戏'));
      await tester.pumpAndSettle();
      await tester.enterText(find.byType(TextField).first, '未匹配');
      await tester.pumpAndSettle();
      await tester.tap(find.text('未匹配游戏'));
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(TextButton, '设置'));
      await tester.pumpAndSettle();
      expect(find.text('导入脚本来源'), findsOneWidget);
      await tester.tap(find.widgetWithText(ListTile, '缓存管理'));
      await tester.pumpAndSettle();
      expect(find.text('2.00 KB'), findsOneWidget);
      await confirm(tester);
      expect(client.cleans, 1);
      expect(find.text('正在清理…'), findsOneWidget);
      expect(
        tester
            .widget<FilledButton>(find.widgetWithText(FilledButton, '正在清理…'))
            .onPressed,
        isNull,
      );
      await tester.tap(find.widgetWithText(ListTile, '关于'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('返回游戏'));
      await tester.pumpAndSettle();
      expect(
        tester
            .widget<EditableText>(find.byType(EditableText).first)
            .controller
            .text,
        '未匹配',
      );
      expect(find.text('游戏信息'), findsOneWidget);
      client.pending!.complete(client.cleanupResult);
      await tester.pumpAndSettle();
      final check = update.check(manual: true);
      update.complete('available');
      await check;
      await tester.pumpAndSettle();
      expect(find.text('发现新版本 v0.4.0'), findsOneWidget);
      await tester.tap(find.widgetWithText(TextButton, '设置'));
      await tester.pumpAndSettle();
      expect(find.text('当前版本 0.3.0'), findsOneWidget);
      expect(find.text('发现新版本 v0.4.0'), findsOneWidget);
      await tester.tap(find.widgetWithText(ListTile, '缓存管理'));
      await tester.pumpAndSettle();
      await tester.ensureVisible(find.text('清理完成，已释放 1.00 KB'));
      expect(find.text('清理完成，已释放 1.00 KB'), findsOneWidget);
      expect(client.stats, 2);
      expect(client.cleans, 1);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox.shrink());
      model.dispose();
      update.dispose();
    },
  );

  testWidgets(
    'partial inventory, confirmation cancellation and truthful refresh errors',
    (tester) async {
      final client = CacheClient();
      client.inventory['diagnostics'] = [
        {'code': 'cleanup_failed', 'detail': 'locked file'},
      ];
      final model = AppModel(client);
      await tester.pumpWidget(
        PatchHubApp(
          home: Scaffold(body: CachePage(model: model.cache)),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.textContaining('部分文件无法统计'), findsOneWidget);
      await tester.ensureVisible(find.widgetWithText(FilledButton, '清理缓存'));
      await tester.tap(find.widgetWithText(FilledButton, '清理缓存'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('取消'));
      await tester.pumpAndSettle();
      expect(client.cleans, 0);
      client.statError = const CoreError('disk_failure', 'disk offline');
      await confirm(tester);
      await tester.ensureVisible(find.text('清理完成，已释放 1.00 KB'));
      expect(find.text('清理完成，已释放 1.00 KB'), findsOneWidget);
      expect(find.text('占用读取失败'), findsOneWidget);
      expect(find.text('暂无需要清理的缓存'), findsNothing);
      await tester.pumpWidget(const SizedBox.shrink());
      model.dispose();
    },
  );

  for (final status in ['partial', 'failed', 'busy']) {
    testWidgets('cleanup $status exposes actual result and manual retry', (
      tester,
    ) async {
      final client = CacheClient();
      client.cleanupResult = {
        'status': status,
        'performed': status != 'busy',
        'removed_bytes': 512,
        'failures': [
          {'code': 'cleanup_failed', 'detail': 'locked file'},
        ],
      };
      final model = AppModel(client);
      await tester.pumpWidget(
        PatchHubApp(
          home: Scaffold(body: CachePage(model: model.cache)),
        ),
      );
      await tester.pumpAndSettle();
      await confirm(tester);
      await tester.ensureVisible(find.text('重试清理'));
      expect(find.text('重试清理'), findsOneWidget);
      expect(
        find.textContaining(status == 'busy' ? '有安装任务正在使用缓存' : '512 B'),
        findsOneWidget,
      );
      await tester.tap(find.text('未清理项详情'));
      await tester.pumpAndSettle();
      expect(find.textContaining('locked file'), findsOneWidget);
      expect(client.cleans, 1);
      await tester.pumpWidget(const SizedBox.shrink());
      model.dispose();
    });
  }

  for (final brightness in Brightness.values) {
    for (final size in [const Size(1180, 760), const Size(850, 560)]) {
      testWidgets(
        'settings in $brightness at $size with scaled text and keyboard focus',
        (tester) async {
          tester.view.physicalSize = size;
          tester.view.devicePixelRatio = 1;
          tester.platformDispatcher.textScaleFactorTestValue = 1.5;
          tester.platformDispatcher.platformBrightnessTestValue = brightness;
          addTearDown(tester.view.resetPhysicalSize);
          addTearDown(tester.view.resetDevicePixelRatio);
          addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
          addTearDown(
            tester.platformDispatcher.clearPlatformBrightnessTestValue,
          );
          final client = CacheClient();
          final model = AppModel(client);
          final update = PanelUpdate()
            ..result = {'state': 'available', 'tag': 'v0.4.0'};
          await tester.pumpWidget(
            PatchHubApp(
              home: AppUpdatePanel(
                update: update,
                lifecycle: AppLifecycle(),
                child: LibraryPage(model: model),
              ),
            ),
          );
          await tester.pumpAndSettle();
          await tester.tap(find.widgetWithText(TextButton, '设置'));
          await tester.pumpAndSettle();
          for (final label in ['缓存管理', '关于', '全局来源']) {
            await tester.tap(find.widgetWithText(ListTile, label));
            await tester.pumpAndSettle();
            await tester.sendKeyEvent(LogicalKeyboardKey.tab);
            await tester.pump();
            expect(tester.takeException(), isNull);
          }
          expect(
            find.byKey(const ValueKey('app-update-banner')),
            findsOneWidget,
          );
          await tester.pumpWidget(const SizedBox.shrink());
          model.dispose();
          update.dispose();
        },
      );
    }
  }

  test(
    'busy core error and empty inventory stay distinct from errors',
    () async {
      final client = CacheClient()
        ..cleanError = const CoreError('installer_busy');
      final model = AppModel(client);
      await model.cache.cleanup();
      expect(cacheError(model.cache.cleanupError!), contains('安装任务'));
      expect(model.cache.result, isNull);
      client.inventory = {'total_bytes': 0, 'entries': 0, 'diagnostics': []};
      await model.cache.refresh();
      expect(model.cache.inventory!['entries'], 0);
      expect(model.cache.inventoryError, isNull);
      model.dispose();
    },
  );
}
