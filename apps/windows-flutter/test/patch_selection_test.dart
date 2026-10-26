import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/app.dart';
import 'package:patchhub_windows/pages/library_page.dart';
import 'package:patchhub_windows/pages/patch_management_page.dart';
import 'package:patchhub_windows/presentation/library_presentation.dart';

import 'modern_layout_test.dart' show ModernFixtureModel;

void main() {
  testWidgets('candidate transitions retain selection; removal still warns', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1280, 900);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final model = ModernFixtureModel();
    final row = model.snapshot!.rows.single;
    final basePatch = {...row.patches.single};
    final corpus = jsonDecode(
      File('../../packages/contracts/fixtures/v5/cases.json')
          .readAsStringSync(),
    );
    final candidate =
        (corpus['events'] as List).firstWhere(
              (item) => item['name'] == 'match_update',
            )['event']['candidates'][0]
            as Map<String, dynamic>;
    final candidates = [
      for (final id in ['other', 'selected'])
        {
          ...candidate,
          'candidate_id': id,
          'patch_title': id,
          'match_basis': 'appid',
        },
    ];
    row.value['discovery'] = {
      'status': 'publisher_match',
      'candidates': candidates,
    };
    row.patches.clear();
    Map<String, dynamic> patch(String id) => {
      ...basePatch,
      'source_id': candidate['source_id'],
      'candidate_id': id,
      'selection_id': 'download-$id',
    };
    await tester.pumpWidget(PatchHubApp(home: LibraryPage(model: model)));
    await tester.pumpAndSettle();
    await tester.tap(find.text(row.name));
    await tester.pumpAndSettle();
    await tester.ensureVisible(find.text('查看条件与来源').last);
    await tester.tap(find.text('查看条件与来源').last);
    await tester.pumpAndSettle();

    const notice = '原补丁已不在当前来源结果中，请重新核对选择。';
    String selected() => tester
        .widget<PatchManagementPage>(find.byType(PatchManagementPage))
        .entry
        .selectionKey;
    final expected = 'candidate:${candidate['source_id']}:selected';
    expect(selected(), expected);

    // The selected candidate is not first, so resetting would select another patch.
    row.patches.addAll([patch('other'), patch('selected')]);
    model.notifyListeners();
    await tester.pumpAndSettle();
    expect(selected(), expected);
    expect(find.text(notice), findsNothing);
    expect(
      tester
          .widget<PatchManagementPage>(find.byType(PatchManagementPage))
          .patch?['selection_id'],
      'download-selected',
    );

    row.patches.removeLast();
    model.notifyListeners();
    await tester.pumpAndSettle();
    expect(selected(), expected);
    expect(find.text(notice), findsNothing);

    candidates.removeLast();
    model.notifyListeners();
    await tester.pumpAndSettle();
    expect(selected(), 'candidate:${candidate['source_id']}:other');
    expect(find.text(notice), findsOneWidget);
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox.shrink());
    model.dispose();
  });

  test(
    'selection identity distinguishes sources and retains legacy releases',
    () {
      final a = DisplayEntry.discovery('same', {
        'source_id': 'a',
        'candidate_id': 'same',
      });
      final b = DisplayEntry.discovery('same', {
        'source_id': 'b',
        'candidate_id': 'same',
      });
      expect(a.selectionKey, isNot(b.selectionKey));
      expect(
        DisplayEntry.installable('release', {}, []).selectionKey,
        'release',
      );
    },
  );
}
