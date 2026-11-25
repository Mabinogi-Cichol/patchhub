import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/app.dart';
import 'package:patchhub_windows/models/app_model.dart';
import 'package:patchhub_windows/pages/settings_page.dart';

import 'library_page_test.dart' show FixtureClient;

void main() {
  testWidgets(
    'Remote settings retain the application listener across navigation',
    (tester) async {
      final model = AppModel(FixtureClient());
      await tester.runAsync(() => model.remote.start('127.0.0.1', port: 0));
      final url = model.remote.url!;
      final key = model.remote.accessKey!;
      String? copied;
      String? opened;
      final messenger = tester.binding.defaultBinaryMessenger;
      messenger.setMockMethodCallHandler(
        const MethodChannel('plugins.flutter.io/url_launcher'),
        (call) async {
          expect(call.method, 'launch');
          opened = (call.arguments as Map)['url'] as String;
          return true;
        },
      );
      messenger.setMockMethodCallHandler(SystemChannels.platform, (call) async {
        if (call.method == 'Clipboard.setData') {
          copied = (call.arguments as Map)['text'] as String;
        }
        return null;
      });
      await tester.pumpWidget(
        PatchHubApp(
          home: Scaffold(body: SettingsPage(model: model)),
        ),
      );
      await tester.tap(find.widgetWithText(ListTile, 'Remote'));
      await tester.pumpAndSettle();
      expect(find.text(url), findsOneWidget);
      expect(find.text(key), findsOneWidget);
      expect(find.text('打开网页'), findsOneWidget);
      final pin = tester.widget<EditableText>(find.text(key));
      expect(pin.style.fontWeight, FontWeight.w700);
      final openButton = find.ancestor(
        of: find.text('打开网页'),
        matching: find.byWidgetPredicate((widget) => widget is FilledButton),
      );
      final copyButton = find.ancestor(
        of: find.text('复制密钥'),
        matching: find.byWidgetPredicate((widget) => widget is FilledButton),
      );
      expect(
        tester.getSize(openButton).width,
        tester.getSize(copyButton).width,
      );
      expect(
        tester.getTopLeft(openButton).dy,
        tester.getTopLeft(copyButton).dy,
      );
      await tester.ensureVisible(find.text('打开网页'));
      await tester.tap(find.text('打开网页'));
      await tester.pumpAndSettle();
      expect(opened, url);
      await tester.ensureVisible(find.text('复制密钥'));
      await tester.tap(find.text('复制密钥'));
      await tester.pumpAndSettle();
      expect(copied, key);
      expect(find.text('已复制密钥'), findsOneWidget);
      expect(find.text('正在监听，离开此页后仍可访问'), findsOneWidget);
      await tester.tap(find.widgetWithText(ListTile, '关于'));
      await tester.pumpAndSettle();
      expect(model.remote.enabled, true);
      await tester.tap(find.widgetWithText(ListTile, 'Remote'));
      await tester.pumpAndSettle();
      expect(find.text(url), findsOneWidget);
      expect(model.remote.accessKey, key);
      await tester.runAsync(model.remote.stop);
      await tester.pumpAndSettle();
      expect(find.text(url), findsNothing);
      expect(find.text(key), findsNothing);
      expect(find.text('打开网页'), findsNothing);
      expect(find.text('复制密钥'), findsNothing);
      expect(find.text('已关闭'), findsOneWidget);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox.shrink());
      model.dispose();
    },
  );
}
