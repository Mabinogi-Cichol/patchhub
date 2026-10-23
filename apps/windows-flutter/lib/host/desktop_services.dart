import 'dart:io';

import 'package:file_selector/file_selector.dart';
import 'package:url_launcher/url_launcher.dart';

class DesktopServices {
  const DesktopServices();
  Future<void> openGameDirectory(String path) async {
    if (!await Directory(path).exists()) {
      throw FileSystemException('游戏目录不存在或无法访问', path);
    }
    if (!await launchUrl(Uri.directory(path, windows: true))) {
      throw FileSystemException('无法打开游戏目录', path);
    }
  }

  Future<String?> selectArchive() async {
    final file = await openFile(
      acceptedTypeGroups: [
        const XTypeGroup(label: 'ZIP 补丁', extensions: ['zip']),
      ],
    );
    return file?.path;
  }

  Future<void> openSource(Uri uri) async {
    if (uri.scheme != 'https') {
      throw ArgumentError.value(uri, 'uri', '来源网页必须使用 HTTPS');
    }
    if (!await launchUrl(uri, mode: LaunchMode.externalApplication)) {
      throw StateError('无法打开来源网页：$uri');
    }
  }
}
