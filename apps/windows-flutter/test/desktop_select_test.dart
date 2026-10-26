import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/app.dart';
import 'package:patchhub_windows/presentation/desktop_select.dart';

void main() {
  for (final reducedMotion in [false, true]) {
    testWidgets(
      'menu animates opening and closing, reduced motion=$reducedMotion',
      (tester) async {
        tester.platformDispatcher.accessibilityFeaturesTestValue =
            FakeAccessibilityFeatures(disableAnimations: reducedMotion);
        addTearDown(
          tester.platformDispatcher.clearAccessibilityFeaturesTestValue,
        );
        await tester.pumpWidget(
          PatchHubApp(
            home: Scaffold(
              body: Center(
                child: SizedBox(
                  width: 360,
                  child: DesktopSelect(
                    label: '补丁选择',
                    value: 'one',
                    options: const [
                      (value: 'one', label: '当前补丁'),
                      (value: 'two', label: '另一款补丁'),
                    ],
                    onSelected: (_) {},
                  ),
                ),
              ),
            ),
          ),
        );
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(const ValueKey('desktop-select-field')));
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 16));
        if (reducedMotion) {
          expect(
            tester
                .widgetList<FadeTransition>(find.byType(FadeTransition))
                .any(
                  (fade) => fade.opacity.value > 0 && fade.opacity.value < 1,
                ),
            isFalse,
          );
          expect(
            tester
                .widget<AnimatedRotation>(find.byType(AnimatedRotation))
                .duration,
            Duration.zero,
          );
        } else {
          expect(tester.binding.transientCallbackCount, greaterThan(0));
          expect(
            tester
                .widgetList<FadeTransition>(find.byType(FadeTransition))
                .any(
                  (fade) => fade.opacity.value > 0 && fade.opacity.value < 1,
                ),
            isTrue,
          );
        }
        await tester.pumpAndSettle();
        expect(
          find.widgetWithText(MenuItemButton, '当前补丁').hitTestable(),
          findsOneWidget,
        );
        await tester.sendKeyEvent(LogicalKeyboardKey.escape);
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 16));
        if (!reducedMotion) {
          expect(tester.binding.transientCallbackCount, greaterThan(0));
        }
        await tester.pumpAndSettle();
        expect(find.byType(MenuItemButton), findsNothing);
        await tester.tap(find.byKey(const ValueKey('desktop-select-field')));
        await tester.pumpAndSettle();
        await tester.tapAt(const Offset(8, 8));
        await tester.pumpAndSettle();
        expect(find.byType(MenuItemButton), findsNothing);
        expect(tester.takeException(), isNull);
      },
    );
  }
  for (final brightness in Brightness.values) {
    for (final bottom in [false, true]) {
      for (final scale in [1.0, 1.5]) {
        testWidgets(
          '$brightness selector at bottom=$bottom scale=$scale keeps anchor visible',
          (tester) async {
            tester.platformDispatcher.textScaleFactorTestValue = scale;
            addTearDown(
              tester.platformDispatcher.clearTextScaleFactorTestValue,
            );
            tester.view.physicalSize = const Size(850, 560);
            tester.view.devicePixelRatio = 1;
            tester.platformDispatcher.platformBrightnessTestValue = brightness;
            addTearDown(tester.view.resetPhysicalSize);
            addTearDown(tester.view.resetDevicePixelRatio);
            addTearDown(
              tester.platformDispatcher.clearPlatformBrightnessTestValue,
            );
            var selected = 'one';
            await tester.pumpWidget(
              PatchHubApp(
                home: Scaffold(
                  body: Align(
                    alignment: bottom
                        ? Alignment.bottomCenter
                        : Alignment.topCenter,
                    child: Padding(
                      padding: const EdgeInsets.all(24),
                      child: SizedBox(
                        width: 380,
                        child: StatefulBuilder(
                          builder: (context, setState) => DesktopSelect(
                            label: '补丁选择',
                            value: selected,
                            options: const [
                              (value: 'one', label: '当前补丁'),
                              (value: 'two', label: '另一款补丁'),
                              (
                                value: 'three',
                                label: 'Ideology in Friction Append Patch · 附加内容与完整汉化补丁下载项',
                              ),
                            ],
                            onSelected: (value) =>
                                setState(() => selected = value),
                          ),
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            );
            await tester.pumpAndSettle();
            final anchor = find.byKey(const ValueKey('desktop-select-field'));
            final anchorRect = tester.getRect(anchor);
            await tester.tap(anchor);
            await tester.pumpAndSettle();
            final item = find
                .widgetWithText(MenuItemButton, '当前补丁')
                .hitTestable();
            expect(item, findsOneWidget);
            final menu = find
                .ancestor(of: item, matching: find.byType(Material))
                .first;
            expect(tester.getRect(menu).overlaps(anchorRect), isFalse);
            expect(
              find.descendant(of: item, matching: find.byIcon(Icons.check)),
              findsOneWidget,
            );
            await tester.tap(
              find.widgetWithText(MenuItemButton, '另一款补丁').hitTestable(),
            );
            await tester.pumpAndSettle();
            expect(selected, 'two');
            expect(find.byType(MenuItemButton).hitTestable(), findsNothing);
            expect(
              find.descendant(of: anchor, matching: find.text('另一款补丁')),
              findsOneWidget,
            );
            await tester.tap(anchor);
            await tester.pumpAndSettle();
            await tester.sendKeyEvent(LogicalKeyboardKey.escape);
            await tester.pumpAndSettle();
            expect(find.byType(MenuItemButton).hitTestable(), findsNothing);
            expect(selected, 'two');
            await tester.sendKeyEvent(LogicalKeyboardKey.tab);
            await tester.pumpAndSettle();
            await tester.sendKeyEvent(LogicalKeyboardKey.space);
            await tester.pumpAndSettle();
            expect(find.byType(MenuItemButton).hitTestable(), findsWidgets);
            expect(
              tester
                  .widget<MenuItemButton>(
                    find.widgetWithText(MenuItemButton, '另一款补丁').hitTestable(),
                  )
                  .focusNode!
                  .hasPrimaryFocus,
              isTrue,
            );
            await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
            await tester.sendKeyEvent(LogicalKeyboardKey.enter);
            await tester.pumpAndSettle();
            expect(selected, 'three');
            expect(find.byType(MenuItemButton).hitTestable(), findsNothing);
            expect(tester.takeException(), isNull);
          },
        );
      }
    }
  }
}
