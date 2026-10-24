import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/host/core_client.dart';

void main() {
  final source = File('../../packages/contracts/generated/v5.schema.json')
      .readAsStringSync();
  final asset = File('assets/protocol-v5.schema.json').readAsStringSync();
  final protocol = WireProtocol(jsonDecode(asset) as Map<String, dynamic>);
  final cases = jsonDecode(
    File('../../packages/contracts/fixtures/v5/cases.json').readAsStringSync(),
  );
  test(
    'bundled schema is the authoritative generated schema',
    () => expect(asset, source),
  );
  for (final item in cases['responses'] as List) {
    test('response ${item['name']}', () {
      final response = item['response'];
      final expected =
          item['error'] ??
          (response['ok'] == false ? response['error']['code'] : null);
      if (expected == null) {
        protocol.response(response, 'fixture', item['method'] as String);
      } else {
        expect(
          () =>
              protocol.response(response, 'fixture', item['method'] as String),
          throwsA(
            isA<CoreError>().having((error) => error.code, 'code', expected),
          ),
        );
      }
    });
  }
  for (final item in cases['events'] as List) {
    test('event ${item['name']}', () {
      if (item['error'] == null) {
        protocol.event(item['event'] as Map<String, dynamic>, 'fixture');
      } else {
        expect(
          () =>
              protocol.event(item['event'] as Map<String, dynamic>, 'fixture'),
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
}
