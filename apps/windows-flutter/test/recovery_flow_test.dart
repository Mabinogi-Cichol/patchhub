import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/app.dart';
import 'package:patchhub_windows/models/app_model.dart';
import 'package:patchhub_windows/pages/records_page.dart';

import 'library_page_test.dart' show FixtureClient;

class RecoveryClient extends FixtureClient {
  final records = <Map<String, dynamic>>[
    {
      'instance_id': 'one',
      'release_id': 'patch-one',
      'transaction_id': 'tx-one',
      'state': 'needs_recovery',
      'updated_at': 0,
    },
    {
      'instance_id': 'other-game',
      'release_id': 'patch-two',
      'transaction_id': 'tx-other',
      'state': 'applying',
      'updated_at': 0,
    },
  ];
  int recoverCalls = 0;
  @override
  Future<Object?> call(
    String method, [
    Map<String, Object?> args = const {},
  ]) async {
    if (method == 'list_transactions') return records;
    if (method == 'recover') {
      expect(args, isEmpty);
      recoverCalls++;
      this.fail = true;
      return [
        {
          ...records.last,
          'state': 'needs_recovery',
          'error': 'backup_corrupted: explicit detail',
        },
      ];
    }
    return super.call(method, args);
  }
}

void main() {
  testWidgets(
    'recovery confirms all games and keeps failed per-item result after refresh error',
    (tester) async {
      final client = RecoveryClient();
      final model = AppModel(client);
      await model.refresh();
      final row = model.snapshot!.rows.single;
      await tester.pumpWidget(
        PatchHubApp(
          home: Scaffold(
            body: RecordsPage(model: model, row: row),
          ),
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('恢复全部未完成操作'));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 300));
      expect(find.textContaining('其它游戏'), findsOneWidget);
      expect(find.textContaining('other-game'), findsOneWidget);
      await tester.tap(find.text('取消'));
      await tester.pumpAndSettle();
      expect(client.recoverCalls, 0);
      await tester.tap(find.text('恢复全部未完成操作'));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 300));
      await tester.tap(find.widgetWithText(FilledButton, '恢复全部未完成操作'));
      await tester.pumpAndSettle();
      expect(client.recoverCalls, 1);
      expect(
        find.textContaining('backup_corrupted: explicit detail'),
        findsOneWidget,
      );
      expect(find.text('恢复结果已返回，状态刷新失败；请重新扫描确认。'), findsOneWidget);
      expect(model.transactions.where(needsRecovery).length, 2);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox.shrink());
      model.dispose();
    },
  );
}
