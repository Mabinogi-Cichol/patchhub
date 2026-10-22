import 'package:file_selector/file_selector.dart';
import 'package:url_launcher/url_launcher.dart';

class DesktopServices {
  const DesktopServices();
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
