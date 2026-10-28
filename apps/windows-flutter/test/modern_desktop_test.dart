import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/app.dart';

import 'installation_flow_test.dart'
    show FixtureDesktop, InstallFixtureModel, mountPatch;

class DirectoryDesktop extends FixtureDesktop {
  String? opened;
  bool fail = false;
  @override
  Future<void> openGameDirectory(String path) async {
    if (fail) throw StateError('目录已被移动');
    opened = path;
  }
}

void main() {
  for (final scenario in [
    'installed',
    'confirmation',
    'archive',
    'blocked',
    'installed_blocked',
  ]) {
    testWidgets('patch summary follows real action conditions: $scenario', (
      tester,
    ) async {
      final model = InstallFixtureModel('success');
      final patch = model.snapshot!.rows.single.patches.single;
      final expected = switch (scenario) {
        'installed' => '已安装补丁',
        'confirmation' => '可安装补丁',
        'archive' => '暂不可安装',
        _ => '暂不可安装',
      };
      switch (scenario) {
        case 'installed':
          patch['detection']['status'] = 'installed_managed';
        case 'confirmation':
          patch['compatibility'] = 'manual_selection_required';
        case 'archive':
          patch['actions']['primary']['input'] = 'unavailable';
          patch['actions']['automatic']['available'] = false;
        case 'blocked':
          patch['actions']['automatic']['available'] = false;
        case 'installed_blocked':
          patch['detection']['status'] = 'installed_managed';
          patch['actions']['automatic']['available'] = false;
      }
      await mountPatch(tester, model, FixtureDesktop());
      final panel = find.byKey(const ValueKey('patch-status-panel'));
      expect(
        find.descendant(of: panel, matching: find.text(expected)),
        findsOneWidget,
      );
      final button = tester.widget<FilledButton>(find.byType(FilledButton));
      expect(
        button.onPressed != null,
        scenario == 'installed' || scenario == 'confirmation',
      );
      expect(find.byType(CheckboxListTile), findsNothing);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox.shrink());
      model.dispose();
    });
  }
  testWidgets(
    'directory action uses the selected instance and explains failure without installing',
    (tester) async {
      final model = InstallFixtureModel('success');
      final desktop = DirectoryDesktop();
      await mountPatch(tester, model, desktop);
      await tester.tap(find.text('打开游戏目录'));
      await tester.pumpAndSettle();
      expect(desktop.opened, model.snapshot!.rows.single.game['install_path']);
      desktop.fail = true;
      await tester.tap(find.text('打开游戏目录'));
      await tester.pump();
      expect(find.textContaining('目录已被移动'), findsOneWidget);
      expect(model.tasks.installation, isNull);
      await tester.pumpWidget(const SizedBox.shrink());
      model.dispose();
    },
  );

  for (final brightness in Brightness.values) {
    testWidgets('$brightness: primary and body text meet design contrast', (
      tester,
    ) async {
      tester.platformDispatcher.platformBrightnessTestValue = brightness;
      addTearDown(tester.platformDispatcher.clearPlatformBrightnessTestValue);
      await tester.pumpWidget(
        const PatchHubApp(home: Scaffold(body: Text('contrast'))),
      );
      final scheme = Theme.of(tester.element(find.text('contrast')))
          .colorScheme;
      double contrast(Color a, Color b) {
        final values = [a.computeLuminance(), b.computeLuminance()]..sort();
        return (values[1] + 0.05) / (values[0] + 0.05);
      }

      expect(
        contrast(scheme.primary, scheme.onPrimary),
        greaterThanOrEqualTo(4.5),
      );
      expect(
        contrast(scheme.surface, scheme.onSurface),
        greaterThanOrEqualTo(4.5),
      );
      expect(
        contrast(scheme.surfaceContainer, scheme.onSurfaceVariant),
        greaterThanOrEqualTo(4.5),
      );
      if (brightness == Brightness.dark) {
        final theme = Theme.of(tester.element(find.text('contrast')));
        for (final color in [
          theme.scaffoldBackgroundColor,
          scheme.surface,
          scheme.surfaceDim,
          scheme.surfaceBright,
          scheme.surfaceContainerLowest,
          scheme.surfaceContainerLow,
          scheme.surfaceContainer,
          scheme.surfaceContainerHigh,
          scheme.surfaceContainerHighest,
          scheme.primaryContainer,
          scheme.secondaryContainer,
          scheme.onSurface,
          scheme.onSurfaceVariant,
          scheme.outline,
          scheme.outlineVariant,
        ]) {
          expect(color.r, closeTo(color.g, 1 / 255));
          expect(color.g, closeTo(color.b, 1 / 255));
        }
        expect(
          theme.scaffoldBackgroundColor.computeLuminance(),
          lessThan(scheme.surface.computeLuminance()),
        );
        expect(
          scheme.surface.computeLuminance(),
          lessThan(scheme.surfaceContainer.computeLuminance()),
        );
        for (final background in [
          scheme.surfaceContainerLowest,
          scheme.surfaceContainerLow,
          scheme.surfaceContainerHigh,
          scheme.surfaceContainerHighest,
          scheme.primaryContainer,
        ]) {
          expect(
            contrast(background, scheme.onSurfaceVariant),
            greaterThanOrEqualTo(4.5),
          );
        }
      }
    });
  }
}
