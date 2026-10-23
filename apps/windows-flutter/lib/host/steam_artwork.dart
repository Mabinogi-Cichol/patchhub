import 'dart:io';

import 'package:flutter/services.dart';

class SteamArtwork {
  const SteamArtwork({
    this.hero,
    this.cover,
    this.thumbnail,
    this.diagnostics = const [],
    this.revision = 0,
  });
  final String? hero;
  final String? cover;
  final String? thumbnail;
  final List<String> diagnostics;
  final int revision;
}

/// Reads only the selected Steam account and the requested app's image folder.
class SteamArtworkStore {
  SteamArtworkStore({Future<Map<String, dynamic>?> Function()? context})
    : _context = context ?? _nativeContext;

  final Future<Map<String, dynamic>?> Function() _context;
  Future<({String? root, String? user, List<String> diagnostics})>? _location;
  final _apps = <int, Future<SteamArtwork>>{};
  int _revision = 0;

  static Future<Map<String, dynamic>?> _nativeContext() =>
      const MethodChannel('steam_patch/desktop')
          .invokeMapMethod<String, dynamic>('steamArtworkContext');

  Future<SteamArtwork> load(int appid) =>
      _apps.putIfAbsent(appid, () => _load(appid));

  void refresh() {
    _revision++;
    _apps.clear();
    _location = null;
  }

  Future<({String? root, String? user, List<String> diagnostics})>
  _locate() async {
    final diagnostics = <String>[];
    String? root;
    try {
      final value = await _context();
      root = value?['root'] as String?;
      if (root == null) {
        return (root: null, user: null, diagnostics: diagnostics);
      }
      final active = value?['activeUser'] as int? ?? 0;
      if (active != 0) {
        return (root: root, user: '$active', diagnostics: diagnostics);
      }
      // Steam uses a 64-bit ID here, while userdata directories use account IDs.
      final login = File('$root/config/loginusers.vdf');
      if (await login.exists()) {
        final text = await login.readAsString();
        final recent =
            RegExp(r'"(\d{17})"\s*\{((?:"(?:\\.|[^"\\])*"|[^{}"])*)\}')
                .allMatches(text)
                .where(
                  (m) => RegExp(
                    r'"MostRecent"\s*"1"',
                    caseSensitive: false,
                  ).hasMatch(m[2]!),
                )
                .toList();
        if (recent.length == 1) {
          final account = int.parse(recent.single[1]!) - 76561197960265728;
          return (root: root, user: '$account', diagnostics: diagnostics);
        }
      }
      return (root: root, user: null, diagnostics: diagnostics);
    } on MissingPluginException {
      // No Windows host in widget tests or unsupported platforms.
      return (root: null, user: null, diagnostics: diagnostics);
    } on PlatformException catch (error) {
      diagnostics.add('Steam 图片位置读取失败：${error.message}');
      return (root: null, user: null, diagnostics: diagnostics);
    } on FileSystemException catch (error) {
      diagnostics.add('Steam 最近账号读取失败：$error');
      // The root remains usable when account metadata is unavailable.
      return (root: root, user: null, diagnostics: diagnostics);
    }
  }

  Future<SteamArtwork> _load(int appid) async {
    final revision = _revision;
    final location = await (_location ??= _locate());
    final diagnostics = [...location.diagnostics];
    final root = location.root;
    if (root == null) return SteamArtwork(diagnostics: diagnostics);
    final custom = <File>[];
    final defaults = <File>[];
    Future<void> list(
      Directory directory,
      List<File> into, {
      bool recursive = false,
    }) async {
      try {
        if (!await directory.exists()) return;
        await for (final entity in directory.list(recursive: recursive)) {
          if (entity is File) into.add(entity);
        }
      } on FileSystemException catch (error) {
        diagnostics.add('游戏 $appid 图片读取失败：$error');
      }
    }

    if (location.user != null) {
      await list(
        Directory('$root/userdata/${location.user}/config/grid'),
        custom,
      );
    }
    await list(
      Directory('$root/appcache/librarycache/$appid'),
      defaults,
      recursive: true,
    );
    String basename(File file) => file.uri.pathSegments.last.toLowerCase();
    bool image(File file) =>
        RegExp(r'\.(png|jpe?g|webp)$').hasMatch(basename(file));
    custom.removeWhere((file) => !image(file));
    defaults.removeWhere((file) => !image(file));
    defaults.sort((a, b) {
      final locale = (basename(a).contains('_schinese') ? 0 : 1).compareTo(
        basename(b).contains('_schinese') ? 0 : 1,
      );
      return locale != 0 ? locale : a.path.compareTo(b.path);
    });
    String? findCustom(String suffix) => custom
        .where((file) => basename(file).split('.').first == '$appid$suffix')
        .firstOrNull
        ?.uri
        .toFilePath();
    String? findDefault(String stem) => defaults
        .where((file) {
          final name = basename(file).split('.').first;
          return name == stem || name == '${stem}_schinese';
        })
        .firstOrNull
        ?.uri
        .toFilePath();
    final customCover = findCustom('p');
    final customHeader = findCustom('');
    final cover =
        customCover ??
        findDefault('library_600x900') ??
        findDefault('library_capsule');
    final header =
        customHeader ?? findDefault('library_header') ?? findDefault('header');
    return SteamArtwork(
      hero: findCustom('_hero') ?? findDefault('library_hero'),
      cover: cover,
      thumbnail: customHeader ?? customCover ?? cover ?? header,
      diagnostics: diagnostics,
      revision: revision,
    );
  }
}
