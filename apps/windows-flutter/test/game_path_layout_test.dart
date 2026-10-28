import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/app.dart';
import 'package:patchhub_windows/pages/library_page.dart';

import 'modern_layout_test.dart' show ModernFixtureModel;

void main() {
  final installPath =
      r'\\?\F:\Steam Library\steamapps\common\中文游戏目录\'
      '${List.filled(12, 'LongDirectoryWithoutSpaces').join()}';
  const libraryPath = r'\\?\F:\Steam Library\中文游戏库';
  for (final size in [
    const Size(1440, 900),
    const Size(1382, 1169),
    const Size(1180, 760),
    const Size(850, 560),
  ]) {
    for (final scale in [1.0, 1.5]) {
      testWidgets(
        '$size scale $scale: full-width paths grow and remain reachable',
        (tester) async {
          tester.view.physicalSize = size;
          tester.view.devicePixelRatio = 1;
          tester.platformDispatcher.textScaleFactorTestValue = scale;
          addTearDown(tester.view.resetPhysicalSize);
          addTearDown(tester.view.resetDevicePixelRatio);
          addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
          final model = ModernFixtureModel();
          final row = model.snapshot!.rows.single;
          row.game['install_path'] = installPath;
          row.game['library_path'] = libraryPath;
          await tester.pumpWidget(PatchHubApp(home: LibraryPage(model: model)));
          await tester.pumpAndSettle();
          await tester.tap(find.text(row.name));
          await tester.pumpAndSettle();
          final footer = tester.getRect(
            find.byKey(const ValueKey('patch-floating-actions')),
          );
          final info = find.widgetWithText(ExpansionTile, '更多游戏信息');
          final status = find.byKey(const ValueKey('patch-status-panel'));
          expect(find.descendant(of: status, matching: info), findsNothing);
          await tester.ensureVisible(info);
          await tester.pumpAndSettle();
          await tester.tap(find.text('更多游戏信息'));
          await tester.pumpAndSettle();
          final path = find.byKey(const ValueKey('game-install-path'));
          final library = find.byKey(const ValueKey('game-library-path'));
          expect(tester.widget<Text>(path).data, installPath);
          expect(tester.widget<Text>(library).data, libraryPath);
          final paragraph = tester.renderObject<RenderParagraph>(
            find.descendant(of: path, matching: find.byType(RichText)),
          );
          final lastCharacter = paragraph
              .getBoxesForSelection(
                TextSelection(
                  baseOffset: installPath.length - 1,
                  extentOffset: installPath.length,
                ),
              )
              .single;
          expect(paragraph.didExceedMaxLines, isFalse);
          expect(
            lastCharacter.bottom,
            lessThanOrEqualTo(paragraph.size.height),
          );
          expect(lastCharacter.right, lessThanOrEqualTo(paragraph.size.width));
          expect(tester.getSize(path).width, tester.getSize(info).width);
          await Scrollable.ensureVisible(tester.element(library), alignment: 1);
          await tester.pumpAndSettle();
          expect(tester.getRect(library).bottom, lessThanOrEqualTo(footer.top));
          expect(library.hitTestable(), findsOneWidget);

          // Copy through the real selection shortcut, preserving the raw path.
          if (size.width == 1382 && scale == 1) {
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
            await mouse.down(tester.getCenter(library));
            await mouse.up();
            await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
            await tester.sendKeyEvent(LogicalKeyboardKey.keyA);
            await tester.sendKeyEvent(LogicalKeyboardKey.keyC);
            await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
            await tester.pump();
            expect(copied, libraryPath);
            await mouse.removePointer();
          }
          await tester.tap(find.text('操作记录'));
          await tester.pumpAndSettle();
          await tester.tap(find.text('补丁管理'));
          await tester.pumpAndSettle();
          expect(library, findsOneWidget);
          expect(tester.takeException(), isNull);
          await tester.pumpWidget(const SizedBox.shrink());
          model.dispose();
        },
      );
    }
  }
}
