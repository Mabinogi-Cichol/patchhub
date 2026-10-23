import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:steam_patch_windows/app.dart';

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
        const SteamPatchApp(home: Scaffold(body: Text('contrast'))),
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
    });
  }
}
