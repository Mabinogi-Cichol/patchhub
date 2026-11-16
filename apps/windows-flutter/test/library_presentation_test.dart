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
      final entries = displayEntries(row);
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

  test('candidateConditions formats related DLC names and local install status', () {
    final dlcCandidate = {
      'variant': 'dlc',
      'requires_dlc': [1390150],
    };
    expect(
      candidateConditions(dlcCandidate, game: {
        'related_dlc': [
          {'appid': 1390150, 'name': '夏日狂想曲+'},
        ],
        'installed_dlc': [1390150],
      }),
      '扩展 / DLC · 需要 DLC 夏日狂想曲+ (1390150)（已安装）',
    );
    expect(
      candidateConditions(dlcCandidate, game: {
        'related_dlc': [
          {'appid': 1390150, 'name': '夏日狂想曲+'},
        ],
        'installed_dlc': <int>[],
      }),
      '扩展 / DLC · 需要 DLC 夏日狂想曲+ (1390150)（未检测到安装）',
    );
    expect(
      candidateConditions(dlcCandidate, game: {
        'related_dlc': <Map<String, dynamic>>[],
        'installed_dlc': [1390150],
      }),
      '扩展 / DLC · 需要 DLC 1390150（已安装）',
    );
  });
}
