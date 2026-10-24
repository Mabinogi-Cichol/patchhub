import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/models/library.dart';
import 'package:patchhub_windows/presentation/library_presentation.dart';

void main() {
  final cases = jsonDecode(
    File('test/fixtures/presentation.json').readAsStringSync(),
  ) as List;
  for (final item in cases) {
    test('TypeScript parity: ${item['name']}', () {
      final row = GameRow(item['row'] as Map<String, dynamic>);
      final entries = displayEntries(row, item['language'] as String?);
      expect([
        for (final entry in entries)
          {
            'kind': entry.installable ? 'installable' : 'discovery',
            'key': entry.key,
            'title': entry.title,
            'choices': entry.choices
                .map((patch) => patch['selection_id'] ?? patch['release_id'])
                .toList(),
          },
      ], item['entries']);
      expect(row.matched, item['matched']);
      expect(needsAttention(row), item['attention']);
      expect(rowSummary(row), item['summary']);
      for (final filter in GameFilter.values) {
        expect(
          filterGames([row], filter, '').map((row) => row.id).toList(),
          item['filters'][filter.name],
        );
      }
      expect(
        filterGames([row], GameFilter.all, '${row.game['appid']}'),
        hasLength(1),
      );
      expect(filterGames([row], GameFilter.all, 'not-in-library'), isEmpty);
    });
  }
}
