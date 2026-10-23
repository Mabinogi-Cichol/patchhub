import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:steam_patch_windows/host/steam_artwork.dart';

void main() {
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

  SteamArtworkStore store({int activeUser = 12}) => SteamArtworkStore(
    context: () async => {'root': root.path, 'activeUser': activeUser},
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
    'nested default art prefers Chinese and excludes blurred hero',
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
      final images = await store().load(42);
      expect(images.cover, cover);
      expect(images.hero, hero);
      expect(images.thumbnail, cover);
      expect(images.diagnostics, isEmpty);
    },
  );
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
