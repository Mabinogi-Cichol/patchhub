import 'dart:convert';
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/app.dart';
import 'package:patchhub_windows/models/app_model.dart';
import 'package:patchhub_windows/host/core_client.dart';
import 'package:patchhub_windows/host/core_paths.dart';
import 'package:patchhub_windows/pages/library_page.dart';

class FixtureClient extends CoreClient {
  FixtureClient()
    : super(
        CorePaths(binary: '', stateDirectory: '', resourceDirectory: ''),
        WireProtocol(
          jsonDecode(File('assets/protocol-v5.schema.json').readAsStringSync())
              as Map<String, dynamic>,
        ),
      );
  bool fail = false;
  int scans = 0;
  @override
  Future<Map<String, dynamic>> capabilities() async => {};
  @override
  Future<Object?> call(
    String method, [
    Map<String, Object?> args = const {},
  ]) async {
    if (fail) throw const CoreError('scan_failed', '磁盘不可访问');
    if (method != 'scan') return <Map<String, dynamic>>[];
    scans++;
    return {
      'catalog_error': null,
      'diagnostics': [],
      'installs': [
        {
          'game': {
            'instance_id': 'one',
            'appid': 1,
            'name': '未匹配游戏',
            'localized_name': null,
          },
          'patches': [],
          'diagnostics': [],
        },
      ],
    };
  }
}

void main() {
  for (final systemBrightness in Brightness.values) {
    testWidgets('theme mode icon updates with unchanged $systemBrightness', (
      tester,
    ) async {
      tester.platformDispatcher.platformBrightnessTestValue = systemBrightness;
      addTearDown(tester.platformDispatcher.clearPlatformBrightnessTestValue);
      final client = FixtureClient();
      final model = AppModel(client);
      await tester.pumpWidget(PatchHubApp(home: LibraryPage(model: model)));
      await tester.pumpAndSettle();
      await tester.tap(find.text('查看全部游戏'));
      await tester.pumpAndSettle();
      final explicitMode = systemBrightness == Brightness.dark
          ? ThemeMode.dark
          : ThemeMode.light;
      for (final mode in [
        explicitMode,
        ThemeMode.system,
        explicitMode,
        ThemeMode.system,
      ]) {
        await tester.tap(find.byTooltip('外观'));
        await tester.pumpAndSettle();
        await tester.tap(
          find.byWidgetPredicate(
            (widget) =>
                widget is CheckedPopupMenuItem<ThemeMode> &&
                widget.value == mode,
          ),
        );
        await tester.pumpAndSettle();
        expect(
          tester.widget<MaterialApp>(find.byType(MaterialApp)).themeMode,
          mode,
        );
        expect(
          Theme.of(tester.element(find.byType(LibraryPage))).brightness,
          systemBrightness,
        );
        expect(
          find.byKey(const ValueKey('system-theme-icon')),
          mode == ThemeMode.system ? findsOneWidget : findsNothing,
        );
        if (mode != ThemeMode.system) {
          expect(
            find.byIcon(
              systemBrightness == Brightness.dark
                  ? Icons.dark_mode_outlined
                  : Icons.light_mode_outlined,
            ),
            findsOneWidget,
          );
        }
      }
      expect(find.text('未匹配游戏'), findsOneWidget);
      expect(client.scans, 1);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox.shrink());
      model.dispose();
    });
  }
  testWidgets(
    'default matched filter and explicit all choice survive refresh failure',
    (tester) async {
      tester.view.physicalSize = const Size(850, 560);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      final client = FixtureClient();
      final model = AppModel(client);
      await tester.pumpWidget(PatchHubApp(home: LibraryPage(model: model)));
      await tester.pumpAndSettle();
      expect(find.text('未匹配游戏'), findsNothing);
      await tester.tap(find.text('查看全部游戏'));
      await tester.pumpAndSettle();
      expect(find.text('未匹配游戏'), findsOneWidget);
      client.fail = true;
      await tester.tap(find.text('重新扫描游戏库'));
      await tester.pumpAndSettle();
      expect(find.text('未匹配游戏'), findsOneWidget);
      await tester.tap(find.text('查看原因'));
      await tester.pumpAndSettle();
      expect(find.textContaining('磁盘不可访问'), findsOneWidget);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox.shrink());
      model.dispose();
    },
  );
  testWidgets(
    'appearance follows system and switches without resetting library',
    (tester) async {
      await (FontLoader(
        'MaterialIcons',
      )..addFont(rootBundle.load('fonts/MaterialIcons-Regular.otf'))).load();
      tester.platformDispatcher.platformBrightnessTestValue = Brightness.light;
      addTearDown(tester.platformDispatcher.clearPlatformBrightnessTestValue);
      final client = FixtureClient();
      final model = AppModel(client);
      final preview = GlobalKey();
      await tester.pumpWidget(
        RepaintBoundary(
          key: preview,
          child: PatchHubApp(home: LibraryPage(model: model)),
        ),
      );
      await tester.pumpAndSettle();
      Brightness brightness() =>
          Theme.of(tester.element(find.byType(LibraryPage))).brightness;
      expect(
        tester.widget<MaterialApp>(find.byType(MaterialApp)).themeMode,
        ThemeMode.system,
      );
      expect(brightness(), Brightness.light);
      expect(find.byKey(const ValueKey('system-theme-icon')), findsOneWidget);
      expect(
        tester.getSize(find.byKey(const ValueKey('system-theme-icon'))),
        const Size(20, 20),
      );
      final iconRect = tester.getRect(
        find.byKey(const ValueKey('system-theme-icon')),
      );
      await tester.runAsync(() async {
        final boundary =
            preview.currentContext!.findRenderObject()!
                as RenderRepaintBoundary;
        final image = await boundary.toImage();
        final bytes = (await image.toByteData(
          format: ui.ImageByteFormat.rawRgba,
        ))!;
        for (var half = 0; half < 2; half++) {
          var painted = 0;
          for (var y = iconRect.top.toInt(); y < iconRect.bottom.toInt(); y++) {
            for (
              var x = iconRect.left.toInt() + half * 10;
              x < iconRect.left.toInt() + (half + 1) * 10;
              x++
            ) {
              final offset = (y * image.width + x) * 4;
              if (bytes.getUint8(offset) < 200) painted++;
            }
          }
          expect(
            painted,
            greaterThan(5),
            reason: 'Both halves of the system theme icon must paint',
          );
        }
        image.dispose();
      });
      await tester.tap(find.text('查看全部游戏'));
      await tester.pumpAndSettle();
      tester.platformDispatcher.platformBrightnessTestValue = Brightness.dark;
      await tester.pumpAndSettle();
      expect(brightness(), Brightness.dark);
      Future<void> choose(String label) async {
        await tester.tap(find.byTooltip('外观'));
        await tester.pumpAndSettle();
        await tester.tap(
          find.ancestor(
            of: find.text(label),
            matching: find.byType(CheckedPopupMenuItem<ThemeMode>),
          ),
        );
        await tester.pumpAndSettle();
      }

      await choose('浅色');
      expect(brightness(), Brightness.light);
      expect(find.byIcon(Icons.light_mode_outlined), findsOneWidget);
      await choose('深色');
      expect(brightness(), Brightness.dark);
      expect(find.byIcon(Icons.dark_mode_outlined), findsOneWidget);
      tester.platformDispatcher.platformBrightnessTestValue = Brightness.light;
      await tester.pumpAndSettle();
      expect(brightness(), Brightness.dark);
      await choose('跟随系统');
      expect(brightness(), Brightness.light);
      expect(find.byKey(const ValueKey('system-theme-icon')), findsOneWidget);
      expect(find.text('未匹配游戏'), findsOneWidget);
      expect(client.scans, 1);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox.shrink());
      model.dispose();
    },
  );
}
