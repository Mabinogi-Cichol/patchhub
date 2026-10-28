import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/host/steam_artwork.dart';
import 'package:patchhub_windows/models/library.dart';

void main() {
  test('artwork language follows its Steam root even without a localized game name', () {
    final snapshot = ScanSnapshot({
      'installs': [],
      'diagnostics': [],
      'catalog_error': null,
      'steam_languages': {
        r'\\?\C:\Steam': 'japanese',
        r'D:\OtherSteam': 'schinese',
      },
    });
    expect(snapshot.steamLanguage('c:/Steam/'), 'japanese');
    expect(snapshot.steamLanguage(r'D:\OtherSteam'), 'schinese');
    expect(snapshot.steamLanguage(r'E:\Unknown'), isNull);
  });
  late Directory root;
  setUp(
    () async => root = await Directory.systemTemp.createTemp('steam-artwork-'),
  );
  tearDown(() async => root.delete(recursive: true));
  Future<String> file(String name, [String text = 'fixture']) async {
    final target = File('${root.path}/$name');
    await target.parent.create(recursive: true);
    await target.writeAsString(text);
    return target.uri.toFilePath();
  }

  SteamArtworkStore store({int activeUser = 12, String? language}) =>
      SteamArtworkStore(
        context: () async => {'root': root.path, 'activeUser': activeUser},
        languageForRoot: (_) => language,
      );

  test(
    'current account custom art wins; never mixes another account',
    () async {
      final custom = await file('userdata/12/config/grid/42p.png');
      final hero = await file('userdata/12/config/grid/42_hero.jpg');
      await file('userdata/99/config/grid/43p.png');
      await file('appcache/librarycache/42/header_schinese.jpg');
      await file('appcache/librarycache/42/hash/library_600x900.jpg');
      final images = await store().load(42);
      expect(images.cover, custom);
      expect(images.hero, hero);
      expect(images.thumbnail, custom);
      expect((await store().load(43)).cover, isNull);
    },
  );
  test(
    'nested default art follows Steam language and excludes blurred hero',
    () async {
      await file('appcache/librarycache/42/library_600x900.jpg');
      final cover = await file(
        'appcache/librarycache/42/hash/library_600x900_schinese.jpg',
      );
      await file(
        'appcache/librarycache/42/hash/library_hero_blur_schinese.jpg',
      );
      final hero = await file(
        'appcache/librarycache/42/hero-hash/library_hero_schinese.jpg',
      );
      final images = await store(language: 'schinese').load(42);
      expect(images.cover, cover);
      expect(images.hero, hero);
      expect(images.thumbnail, cover);
      expect(images.diagnostics, isEmpty);
    },
  );
  test('Steam language refresh changes all default artwork and falls back to neutral art', () async {
    final neutral = await file('appcache/librarycache/42/library_600x900.jpg');
    final english = await file(
      'appcache/librarycache/42/library_600x900_english.jpg',
    );
    final japanese = await file(
      'appcache/librarycache/42/hash/library_600x900_japanese.jpg',
    );
    final hero = await file(
      'appcache/librarycache/42/library_hero_japanese.jpg',
    );
    await file('appcache/librarycache/42/library_600x900_schinese.jpg');
    var language = 'english';
    final source = SteamArtworkStore(
      context: () async => {'root': root.path, 'activeUser': 12},
      languageForRoot: (_) => language,
    );
    expect((await source.load(42)).cover, english);
    language = 'japanese';
    source.refresh();
    final images = await source.load(42);
    expect(images.cover, japanese);
    expect(images.hero, hero);
    expect(images.thumbnail, japanese);
    expect((await store(language: 'german').load(42)).cover, neutral);
    expect((await store().load(42)).cover, neutral);
    final custom = await file('userdata/12/config/grid/42p.png');
    expect((await store(language: 'japanese').load(42)).cover, custom);
  });
  test('Steam stopped: resolve MostRecent account from loginusers', () async {
    await file(
      'config/loginusers.vdf',
      r'"users" { "76561197960265740" { "PersonaName" "Name {\"quoted\"}" "MostRecent" "1" } }',
    );
    final custom = await file('userdata/12/config/grid/42p.png');
    expect((await store(activeUser: 0).load(42)).cover, custom);
  });
  test(
    'ambiguous account uses defaults and refresh sees newly added art',
    () async {
      await file('userdata/12/config/grid/42p.png');
      final source = store(activeUser: 0);
      expect((await source.load(42)).cover, isNull);
      final cover = await file('appcache/librarycache/42/library_capsule.jpg');
      source.refresh();
      expect((await source.load(42)).cover, cover);
      expect((await source.load(999)).hero, isNull);
    },
  );
}
