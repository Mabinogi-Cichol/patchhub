import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/app.dart';
import 'package:patchhub_windows/pages/records_page.dart';

import 'modern_layout_test.dart' show ModernFixtureModel;

void main() {
  for (final size in [const Size(1337, 955), const Size(850, 560)]) {
    for (final scale in [1.0, 1.5]) {
      for (final brightness in Brightness.values) {
        testWidgets(
          '$size $scale $brightness: records expand independently and copy',
          (tester) async {
            tester.view.physicalSize = size;
            tester.view.devicePixelRatio = 1;
            tester.platformDispatcher.textScaleFactorTestValue = scale;
            tester.platformDispatcher.platformBrightnessTestValue = brightness;
            addTearDown(tester.view.resetPhysicalSize);
            addTearDown(tester.view.resetDevicePixelRatio);
            addTearDown(
              tester.platformDispatcher.clearTextScaleFactorTestValue,
            );
            addTearDown(
              tester.platformDispatcher.clearPlatformBrightnessTestValue,
            );
            final model = ModernFixtureModel();
            final row = model.snapshot!.rows.single;
            final longError = List.filled(30, '文件写入失败：游戏目录中的文件正在使用。').join();
            model.transactions = [
              for (var index = 0; index < 2; index++)
                {
                  'instance_id': row.id,
                  'transaction_id': 'transaction-$index',
                  'plan_id': 'plan-$index',
                  'release_id': 'release-$index',
                  'selection_id': 'selection-$index',
                  'state': index == 0 ? 'completed' : 'failed',
                  'direct': true,
                  'created_at': 1794152836,
                  'updated_at': 1794152836,
                  if (index == 1) 'error': longError,
                },
            ];
            await tester.pumpWidget(
              PatchHubApp(
                home: Scaffold(
                  body: RecordsPage(model: model, row: row),
                ),
              ),
            );
            await tester.pumpAndSettle();
            final first = find.byKey(
              const PageStorageKey('record:transaction-0'),
            );
            final second = find.byKey(
              const PageStorageKey('record:transaction-1'),
            );
            Future<void> toggle(Finder tile) async {
              final header = find.descendant(
                of: tile,
                matching: find.byType(ListTile),
              );
              await tester.ensureVisible(header);
              await tester.pumpAndSettle();
              await tester.tap(header);
              await tester.pumpAndSettle();
              expect(tester.takeException(), isNull);
            }

            await toggle(first);
            final details = find.textContaining('事务：transaction-0');
            expect(details, findsOneWidget);
            expect(tester.getSize(details).height, lessThan(300));
            final expected = tester.widget<Text>(details).data!;
            String? copied;
            tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
              SystemChannels.platform,
              (call) async {
                if (call.method == 'Clipboard.setData') {
                  copied = (call.arguments as Map)['text'] as String;
                }
                return null;
              },
            );
            addTearDown(
              () => tester.binding.defaultBinaryMessenger
                  .setMockMethodCallHandler(SystemChannels.platform, null),
            );
            final mouse = await tester.createGesture(
              kind: PointerDeviceKind.mouse,
            );
            await mouse.down(tester.getCenter(details));
            await mouse.up();
            await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
            await tester.sendKeyEvent(LogicalKeyboardKey.keyA);
            await tester.sendKeyEvent(LogicalKeyboardKey.keyC);
            await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
            await tester.pump();
            expect(copied, expected);
            await mouse.removePointer();
            await toggle(second);
            expect(details, findsOneWidget);
            expect(find.text('具体原因：$longError'), findsOneWidget);
            final lastDetails = find.textContaining('事务：transaction-1');
            await Scrollable.ensureVisible(
              tester.element(lastDetails),
              alignment: 1,
            );
            await tester.pumpAndSettle();
            expect(lastDetails.hitTestable(), findsOneWidget);
            expect(
              tester.getRect(lastDetails).bottom,
              lessThanOrEqualTo(size.height),
            );
            tester
                .state<ScrollableState>(find.byType(Scrollable).first)
                .position
                .jumpTo(0);
            await tester.pumpAndSettle();
            expect(details, findsOneWidget);
            await toggle(first);
            expect(details, findsNothing);
            expect(lastDetails, findsOneWidget);
            await toggle(first);
            expect(details, findsOneWidget);
            expect(tester.takeException(), isNull);
            await tester.pumpWidget(const SizedBox.shrink());
            model.dispose();
          },
        );
      }
    }
  }
}
