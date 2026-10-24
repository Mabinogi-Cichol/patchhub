// Run only with PATCHHUB_UPDATE_FIXTURE pointing to an isolated native bundle.
// Release metadata and HTTPS bodies are local fixtures; production has no test URL.
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/app.dart';
import 'package:patchhub_windows/host/app_lifecycle.dart';
import 'package:patchhub_windows/host/app_update.dart';
import 'package:patchhub_windows/host/core_client.dart';
import 'package:patchhub_windows/host/core_paths.dart';
import 'package:patchhub_windows/host/source_tasks.dart';
import 'package:patchhub_windows/pages/app_update_panel.dart';

class FixtureHttp extends HttpOverrides {
  FixtureHttp(this.root);
  final Directory root;
  @override
  HttpClient createHttpClient(SecurityContext? context) => FixtureClient(root);
}

class FixtureClient implements HttpClient {
  FixtureClient(this.root);
  final Directory root;
  @override
  Future<HttpClientRequest> getUrl(Uri url) async {
    final name = url.pathSegments.last;
    if (url !=
            Uri.parse(
              'https://github.com/$updateRepository/releases/download/v0.3.0/$name',
            ) ||
        ![windowsUpdateAsset, 'SHA256SUMS.txt'].contains(name)) {
      throw StateError('Unexpected fixture URL: $url');
    }
    return FixtureRequest(File('${root.path}/$name'));
  }

  @override
  void close({bool force = false}) {}
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class FixtureRequest implements HttpClientRequest {
  FixtureRequest(this.file);
  final File file;
  @override
  Future<HttpClientResponse> close() async => FixtureResponse(file);
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class FixtureResponse extends Stream<List<int>> implements HttpClientResponse {
  FixtureResponse(this.file);
  final File file;
  @override
  int get contentLength => file.lengthSync();
  @override
  int get statusCode => 200;
  @override
  List<RedirectInfo> get redirects => [];
  @override
  StreamSubscription<List<int>> listen(
    void Function(List<int>)? onData, {
    Function? onError,
    void Function()? onDone,
    bool? cancelOnError,
  }) => file.openRead().listen(
    onData,
    onError: onError,
    onDone: onDone,
    cancelOnError: cancelOnError,
  );
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final path = Platform.environment['PATCHHUB_UPDATE_FIXTURE'];
  testWidgets(
    'confirmed update downloads, validates and hands off to the real Windows updater',
    (tester) async {
      if (path == null) return;
      final root = Directory(path);
      HttpOverrides.global = FixtureHttp(root);
      final closed = Completer<void>();
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        const MethodChannel('window_manager'),
        (call) async {
          if (call.method == 'close' && !closed.isCompleted) closed.complete();
          return null;
        },
      );
      final update = AppUpdate(
        directory: Directory('$path/program'),
        current: '0.2.9',
        stateFile: File(
          '${Platform.environment['APPDATA']}/PatchHub/updates/state.json',
        ),
        fetchRelease: () async =>
            jsonDecode(await File('$path/release.json').readAsString())
                as Map<String, dynamic>,
      );
      final protocol = WireProtocol(
        jsonDecode(File('assets/protocol-v5.schema.json').readAsStringSync())
            as Map<String, dynamic>,
      );
      final tasks = AppTasks(
        CoreClient(
          CorePaths(
            binary: '$path/program/patchhub-core.exe',
            stateDirectory: '$path/state',
            resourceDirectory: '$path/program/resources',
          ),
          protocol,
        ),
      );
      final lifecycle = AppLifecycle(tasks: tasks);
      try {
        await tester.runAsync(update.initialize);
        await lifecycle.initialize();
        await tester.pumpWidget(
          PatchHubApp(
            home: AppUpdatePanel(
              update: update,
              lifecycle: lifecycle,
              child: const Scaffold(body: Center(child: Text('隔离旧版客户端'))),
            ),
          ),
        );
        await tester.pumpAndSettle();
        await tester.tap(find.text('查看更新'));
        await tester.pumpAndSettle();
        await tester.ensureVisible(find.text('下载更新'));
        expect(find.text('下载更新'), findsOneWidget);
        await tester.runAsync(() async {
          await tester.tap(find.text('下载更新'));
          while (update.result['state'] == 'downloading') {
            await Future<void>.delayed(const Duration(milliseconds: 10));
          }
        });
        expect(update.result['state'], 'ready', reason: '${update.result}');
        await tester.pump();
        expect(find.text('重启并更新'), findsOneWidget);
        await tester.runAsync(() async {
          await tester.tap(find.text('重启并更新'));
          while (!closed.isCompleted) {
            if (update.result['state'] == 'failed') {
              throw StateError('${update.result['error']}');
            }
            await Future<void>.delayed(const Duration(milliseconds: 10));
          }
        });
        await tester.runAsync(
          () => File('$path/ui-confirmed.json').writeAsString(
            jsonEncode({
              'downloaded_and_verified': true,
              'confirmed_in_ui': true,
              'normal_close_requested': true,
            }),
            flush: true,
          ),
        );
        await tester.pumpWidget(const SizedBox.shrink());
      } finally {
        lifecycle.dispose();
        update.dispose();
        HttpOverrides.global = null;
      }
    },
    skip: path == null,
    timeout: const Timeout(Duration(minutes: 2)),
  );
}
