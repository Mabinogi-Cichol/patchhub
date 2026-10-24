import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';

const updateRepository = 'Mabinogi-Cichol/steam-hentai-patch';
const windowsUpdateAsset = 'steam-patch-manager-windows-portable.zip';

class AppVersion implements Comparable<AppVersion> {
  AppVersion(String value) {
    final match = RegExp(
      r'^v?(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$',
    ).firstMatch(value);
    if (match == null) throw const FormatException('无法识别版本');
    if (value.contains('+') &&
        value.split('+').last.split('.').any((part) => part.isEmpty)) {
      throw const FormatException('无法识别版本');
    }
    numbers = [for (var i = 1; i <= 3; i++) int.parse(match[i]!)];
    pre = match[4]?.split('.');
    if (pre?.any(
          (s) =>
              s.isEmpty ||
              (int.tryParse(s) != null && s.length > 1 && s.startsWith('0')),
        ) ==
        true) {
      throw const FormatException('无法识别版本');
    }
  }
  late final List<int> numbers;
  late final List<String>? pre;
  @override
  int compareTo(AppVersion other) {
    for (var i = 0; i < 3; i++) {
      final difference = numbers[i].compareTo(other.numbers[i]);
      if (difference != 0) return difference;
    }
    if (pre == null) return other.pre == null ? 0 : 1;
    if (other.pre == null) return -1;
    for (var i = 0; i < pre!.length && i < other.pre!.length; i++) {
      final a = pre![i], b = other.pre![i];
      if (a == b) continue;
      final an = int.tryParse(a), bn = int.tryParse(b);
      if (an != null && bn != null) return an.compareTo(bn);
      if ((an == null) != (bn == null)) return an == null ? 1 : -1;
      return a.compareTo(b);
    }
    return pre!.length.compareTo(other.pre!.length);
  }
}

Map<String, dynamic> updateCandidate(
  Map<String, dynamic> release,
  String current,
) {
  final local = AppVersion(current),
      remote = AppVersion(release['tag_name'] as String);
  if (release['draft'] == true ||
      release['prerelease'] == true ||
      remote.pre != null) {
    throw const FormatException('Release 不是正式版');
  }
  final tag = release['tag_name'] as String;
  final result = <String, dynamic>{
    'current': current,
    'tag': tag,
    'release_id': release['id'],
    'notes': release['body'] ?? '',
    'published_at': release['published_at'],
    'page': 'https://github.com/$updateRepository/releases/tag/$tag',
    'state': 'up_to_date',
  };
  if (remote.compareTo(local) <= 0) return result;
  for (final name in [windowsUpdateAsset, 'SHA256SUMS.txt']) {
    final matches = (release['assets'] as List)
        .cast<Map<String, dynamic>>()
        .where((a) => a['name'] == name)
        .toList();
    if (matches.length != 1) {
      return {...result, 'state': 'unavailable', 'error': '新版已发布，当前平台更新包尚不可用'};
    }
    if (matches.single['browser_download_url'] !=
        'https://github.com/$updateRepository/releases/download/$tag/$name') {
      throw const FormatException('附件不属于官方冻结版本');
    }
    result[name == windowsUpdateAsset ? 'asset' : 'sums'] = matches.single;
  }
  return {...result, 'state': 'available'};
}

class AppUpdate extends ChangeNotifier {
  AppUpdate({
    required this.directory,
    required this.current,
    required this.stateFile,
    this.fetchRelease,
  });
  final Directory directory;
  final String current;
  final File stateFile;
  final Future<Map<String, dynamic>> Function()? fetchRelease;
  Map<String, dynamic> data = {}, result = {'state': 'idle'};
  Future<void>? _pending;
  Timer? _timer;
  HttpClient? _download;
  bool cancelled = false;
  bool lastCheckWasManual = false;
  int bytes = 0, total = 0;
  bool get busy =>
      ['checking', 'downloading', 'applying'].contains(result['state']);
  bool get skipped => data['skipped_tag'] == result['tag'];
  File? package;
  Map<String, dynamic>? outcome;
  Future<void>? _initialization;

  Future<void> initialize() => _initialization ??= _initialize();

  Future<void> _initialize() async {
    final previous = File('${stateFile.parent.path}/last-update-result.json');
    if (await previous.exists()) {
      final value =
          jsonDecode(await previous.readAsString()) as Map<String, dynamic>;
      if (value['target'] == directory.path) outcome = value;
    }
    if (await stateFile.exists()) {
      try {
        data =
            jsonDecode(await stateFile.readAsString()) as Map<String, dynamic>;
      } catch (e) {
        result = {'state': 'failed', 'error': '更新记录读取失败：$e'};
        notifyListeners();
        return;
      }
    }
    result =
        (data['candidate'] as Map?)?.cast<String, dynamic>() ??
        {'state': 'idle'};
    if (result['current'] != current) {
      data.remove('last_attempt_at');
      result = {'state': 'idle', 'current': current};
    }
    await check();
    _timer = Timer.periodic(
      const Duration(hours: 1),
      (_) => unawaited(check()),
    );
  }

  Future<void> _save() async {
    await stateFile.parent.create(recursive: true);
    final temp = File('${stateFile.path}.tmp');
    await temp.writeAsString(jsonEncode(data), flush: true);
    await temp.rename(stateFile.path);
  }

  Future<Map<String, dynamic>> _fetch() async {
    final http = HttpClient();
    try {
      final request = await http.getUrl(
        Uri.parse(
          'https://api.github.com/repos/$updateRepository/releases/latest',
        ),
      );
      request.headers.set('User-Agent', 'steam-patch-manager');
      request.headers.set('Accept', 'application/vnd.github+json');
      request.headers.set('X-GitHub-Api-Version', '2022-11-28');
      final response = await request.close();
      if ((response.statusCode == 403 || response.statusCode == 429) &&
          (response.headers.value('retry-after') != null ||
              response.headers.value('x-ratelimit-remaining') == '0')) {
        final retry = int.tryParse(response.headers.value('retry-after') ?? '');
        data['retry_at'] = retry != null
            ? DateTime.now().millisecondsSinceEpoch ~/ 1000 + retry
            : int.parse(response.headers.value('x-ratelimit-reset')!);
      }
      if (response.statusCode != 200) {
        throw HttpException('GitHub HTTP ${response.statusCode}');
      }
      return jsonDecode(await response.transform(utf8.decoder).join())
          as Map<String, dynamic>;
    } finally {
      http.close(force: true);
    }
  }

  Future<void> check({bool manual = false}) async {
    if (_pending != null) return _pending;
    if (result['state'] == 'downloading' || result['state'] == 'applying') {
      return;
    }
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    final delta = now - (data['last_attempt_at'] as num? ?? 0);
    if (now < (data['retry_at'] as num? ?? 0) ||
        (!manual && delta >= 0 && delta < 86400)) {
      return;
    }
    lastCheckWasManual = manual;
    _pending = _check(now);
    try {
      await _pending;
    } finally {
      _pending = null;
    }
  }

  Future<void> _check(int now) async {
    data['last_attempt_at'] = now;
    result = {'state': 'checking'};
    notifyListeners();
    try {
      AppVersion(current);
      result = updateCandidate(
        await (fetchRelease?.call() ?? _fetch()),
        current,
      );
      data['last_success_at'] = now;
      data.remove('retry_at');
    } catch (e) {
      result = {'state': 'failed', 'current': current, 'error': '$e'};
    }
    data['candidate'] = result;
    try {
      await _save();
    } catch (e) {
      result = {'state': 'failed', 'error': '更新记录保存失败：$e'};
    }
    notifyListeners();
  }

  Future<void> skip() async {
    data['skipped_tag'] = result['tag'];
    await _save();
    notifyListeners();
  }

  void cancelDownload() {
    cancelled = true;
    _download?.close(force: true);
  }

  Future<Directory> prepare() async {
    if (result['state'] != 'ready' || package == null) {
      throw StateError('更新包尚未准备完成');
    }
    final helper = File('${directory.path}/windows-update.ps1');
    if (!await helper.exists()) throw StateError('此构建未包含更新助手，请先手动安装支持自更新的版本');
    final work = await stateFile.parent.createTemp('transaction-');
    final downloadDirectory = package!.parent;
    await package!.rename('${work.path}/package.zip');
    package = null;
    if (await downloadDirectory.list().isEmpty) {
      await downloadDirectory.delete();
    }
    await helper.copy('${work.path}/windows-update.ps1');
    await File('${work.path}/request.json').writeAsString(
      jsonEncode({
        'target': directory.path,
        'version': (result['tag'] as String).replaceFirst(RegExp('^v'), ''),
        'sha256': result['sha256'],
      }),
      flush: true,
    );
    final process = await Process.start('powershell.exe', [
      '-NoProfile',
      '-NonInteractive',
      '-WindowStyle',
      'Hidden',
      '-ExecutionPolicy',
      'Bypass',
      '-File',
      '${work.path}/windows-update.ps1',
      '-Target',
      directory.path,
      '-Work',
      work.path,
      '-ParentId',
      '$pid',
    ]);
    final diagnostic = process.stderr.transform(utf8.decoder).join();
    unawaited(process.stdout.drain<void>());
    int? exitCode;
    unawaited(process.exitCode.then((code) => exitCode = code));
    // A normal Windows child survives its parent; drain both pipes while GUI is alive.
    while (!await File('${work.path}/prepared.json').exists()) {
      final failure = File('${stateFile.parent.path}/last-update-result.json');
      if (await failure.exists()) {
        final saved = jsonDecode(await failure.readAsString()) as Map;
        if (saved['work'] == work.path) throw StateError('${saved['error']}');
      }
      if (exitCode != null) {
        throw StateError('更新助手启动失败：$exitCode\n${await diagnostic}');
      }
      await Future<void>.delayed(const Duration(milliseconds: 100));
    }
    return work;
  }

  Future<void> download() async {
    final candidate = Map<String, dynamic>.from(result);
    if (candidate['state'] != 'available') throw StateError('请重新检查更新');
    cancelled = false;
    bytes = 0;
    package = null;
    result = {...candidate, 'state': 'downloading'};
    notifyListeners();
    Directory? work;
    File? target, sums;
    try {
      await stateFile.parent.create(recursive: true);
      work = await stateFile.parent.createTemp('download-');
      target = File('${work.path}/package.zip');
      sums = File('${work.path}/SHA256SUMS.txt');
      _download = HttpClient();
      // Freeze and revalidate asset identities before downloading executable content.
      final refreshed = updateCandidate(
        await (fetchRelease?.call() ?? _fetch()),
        current,
      );
      for (final key in ['release_id', 'tag']) {
        if (candidate[key] != refreshed[key]) {
          throw StateError('Release 已变化，请重新检查');
        }
      }
      for (final key in ['asset', 'sums']) {
        for (final field in [
          'id',
          'browser_download_url',
          'digest',
          'size',
          'updated_at',
        ]) {
          if (candidate[key][field] != refreshed[key]?[field]) {
            throw StateError('发行附件已变化，请重新检查');
          }
        }
      }
      for (final entry in [
        (candidate['sums'], sums),
        (candidate['asset'], target),
      ]) {
        final request = await _download!.getUrl(
          Uri.parse(entry.$1['browser_download_url'] as String),
        );
        final response = await request.close();
        if (response.statusCode != 200 ||
            response.redirects.any(
              (r) => r.location.hasScheme && r.location.scheme != 'https',
            )) {
          throw HttpException('附件下载失败：${response.statusCode}');
        }
        total = response.contentLength;
        bytes = 0;
        final sink = entry.$2.openWrite();
        try {
          await for (final block in response) {
            if (cancelled) throw StateError('已取消下载');
            sink.add(block);
            bytes += block.length;
            notifyListeners();
          }
        } finally {
          await sink.close();
        }
      }
      if (cancelled) throw StateError('已取消下载');
      final lines = const LineSplitter().convert(await sums.readAsString());
      final pattern = RegExp(
        r'^([0-9a-fA-F]{64}) [ *]' + RegExp.escape(windowsUpdateAsset) + r'$',
      );
      final hashes = lines
          .map(pattern.firstMatch)
          .whereType<RegExpMatch>()
          .map((m) => m[1]!.toLowerCase())
          .toList();
      if (hashes.length != 1) throw const FormatException('校验清单缺少唯一 ZIP 哈希');
      final digest = candidate['asset']['digest'];
      if (digest != null && digest != 'sha256:${hashes.single}') {
        throw const FormatException('附件摘要与校验清单不一致');
      }
      // The trusted, already-installed helper verifies SHA-256 again before extraction.
      final verified = await Process.run(
        'powershell.exe',
        [
          '-NoProfile',
          '-NonInteractive',
          '-Command',
          r'$s=[IO.File]::OpenRead($env:STEAM_PATCH_UPDATE_PACKAGE); $h=[Security.Cryptography.SHA256]::Create(); try { [BitConverter]::ToString($h.ComputeHash($s)).Replace("-", "") } finally { $s.Dispose(); $h.Dispose() }',
        ],
        environment: {'STEAM_PATCH_UPDATE_PACKAGE': target.path},
      );
      if (verified.exitCode != 0 ||
          '${verified.stdout}'.trim().toLowerCase() != hashes.single) {
        throw const FormatException('ZIP SHA-256 不匹配');
      }
      package = target;
      result = {...candidate, 'state': 'ready', 'sha256': hashes.single};
    } catch (e) {
      if (target != null && await target.exists()) await target.delete();
      result = {...candidate, 'state': 'failed', 'error': '$e'};
    } finally {
      _download?.close(force: true);
      _download = null;
      if (sums != null && await sums.exists()) await sums.delete();
      if (work != null && await work.list().isEmpty) await work.delete();
      notifyListeners();
    }
  }

  @override
  void dispose() {
    _timer?.cancel();
    _download?.close(force: true);
    super.dispose();
  }
}
