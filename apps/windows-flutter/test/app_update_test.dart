import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/host/app_update.dart';

void main() {
  test('shared semantic version contract', () {
    final fixture = jsonDecode(
      File('../../scripts/fixtures/app-update.json').readAsStringSync(),
    ) as Map;
    for (final value in fixture['versions'] as List) {
      expect(
        AppVersion(value['remote'] as String)
                .compareTo(AppVersion(value['local'] as String)) >
            0,
        value['newer'],
      );
    }
    for (final value in fixture['invalid'] as List) {
      expect(() => AppVersion(value as String), throwsFormatException);
    }
  });
  test('missing or foreign update assets cannot be installed', () {
    final release = <String, dynamic>{
      'id': 1,
      'tag_name': 'v0.4.0',
      'assets': <Map<String, dynamic>>[],
    };
    expect(updateCandidate(release, '0.3.0')['state'], 'unavailable');
    release['assets'] = [
      for (final name in [windowsUpdateAsset, 'SHA256SUMS.txt'])
        {
          'id': 1,
          'name': name,
          'browser_download_url': 'https://example.com/$name',
        },
    ];
    expect(() => updateCandidate(release, '0.3.0'), throwsFormatException);
  });
  test('concurrent checks coalesce and failures remain visible', () async {
    final root = Directory.systemTemp.createTempSync('app-update-');
    var calls = 0;
    final update = AppUpdate(
      directory: root,
      current: '0.3.0',
      stateFile: File('${root.path}/state.json'),
      fetchRelease: () async {
        calls++;
        await Future<void>.delayed(Duration.zero);
        throw const HttpException('offline');
      },
    );
    try {
      await Future.wait([
        update.check(manual: true),
        update.check(manual: true),
      ]);
      expect(calls, 1);
      expect(update.result['state'], 'failed');
      await update.check();
      expect(calls, 1);
    } finally {
      update.dispose();
      root.deleteSync(recursive: true);
    }
  });

  test(
    'new download clears stale progress before release revalidation',
    () async {
      final root = Directory.systemTemp.createTempSync('app-update-');
      final release = Completer<Map<String, dynamic>>();
      final fetching = Completer<void>();
      final update = AppUpdate(
        directory: root,
        current: '0.3.0',
        stateFile: File('${root.path}/state.json'),
        fetchRelease: () {
          fetching.complete();
          return release.future;
        },
      );
      try {
        update.result = {'state': 'available'};
        update.bytes = 100;
        update.total = 200;
        final download = update.download();
        expect(update.bytes, 0);
        expect(update.total, 0);
        expect(update.downloadLabel, '正在核对发行信息…');
        await fetching.future;
        release.completeError(
          const HandshakeException('Connection terminated during handshake'),
        );
        await download;
        expect(update.result['state'], 'failed');
        expect(update.result['error'], contains('HandshakeException'));
      } finally {
        update.dispose();
        root.deleteSync(recursive: true);
      }
    },
  );
}
