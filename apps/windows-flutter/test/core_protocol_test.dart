import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:steam_patch_windows/host/core_client.dart';
import 'package:steam_patch_windows/models/library.dart';

void main() {
  final corpus = jsonDecode(
    File('../../packages/contracts/fixtures/v5/cases.json').readAsStringSync(),
  ) as Map<String, dynamic>;
  final responses = (corpus['responses'] as List).cast<Map<String, dynamic>>();
  for (final item in responses.where(
    (item) =>
        item['method'] == 'get_capabilities' && item['response']['ok'] == true,
  )) {
    test('v5 ${item['name']}', () {
      final value = item['response']['result'] as Map<String, dynamic>;
      if (item['error'] == null) {
        validateCapabilities(value);
      } else {
        expect(
          () => validateCapabilities(value),
          throwsA(
            isA<CoreError>().having(
              (error) => error.code,
              'code',
              item['error'],
            ),
          ),
        );
      }
    });
  }
  test('scan fixture retains instance identity and matched semantics', () {
    final value = responses.firstWhere(
      (item) => item['name'] == 'scan-success',
    );
    final result = decodeResponse(jsonEncode(value['response']), 'fixture');
    final snapshot = ScanSnapshot(result as Map<String, dynamic>);
    expect(snapshot.rows.single.id, 'instance');
    expect(snapshot.rows.single.matched, isTrue);
    expect(snapshot.rows.single.name, 'Fixture');
  });
  test('unknown errors retain complete detail', () {
    expect(
      () => decodeResponse(
        jsonEncode({
          'protocol_version': 5,
          'request_id': 'fixture',
          'ok': false,
          'error': {'code': 'new_core_error', 'detail': '完整错误\n下一行'},
        }),
        'fixture',
      ),
      throwsA(
        isA<CoreError>()
            .having((error) => error.code, 'code', 'new_core_error')
            .having((error) => error.detail, 'detail', '完整错误\n下一行'),
      ),
    );
  });
  test('wrong request or version cannot become successful empty data', () {
    for (final response in [
      {
        'protocol_version': 4,
        'request_id': 'fixture',
        'ok': true,
        'result': {},
      },
      {'protocol_version': 5, 'request_id': 'other', 'ok': true, 'result': {}},
      {
        'protocol_version': 5,
        'request_id': 'fixture',
        'ok': 'true',
        'result': {},
      },
    ]) {
      expect(
        () => decodeResponse(jsonEncode(response), 'fixture'),
        throwsA(isA<CoreError>()),
      );
    }
  });
}
