import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/host/core_client.dart';
import 'package:patchhub_windows/host/core_paths.dart';
import 'package:patchhub_windows/models/app_model.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  // This integration check uses real loopback HTTP instead of the widget stub.
  HttpOverrides.global = null;
  final binary = File(
    '../../target/debug/patchhub-core${Platform.isWindows ? '.exe' : ''}',
  ).absolute;
  test(
    'HTTP serves packaged assets and persists Lua management through the real core',
    () async {
      final temp = await Directory.systemTemp.createTemp(
        'patchhub-remote-native-',
      );
      final paths = CorePaths(
        binary: binary.path,
        stateDirectory: '${temp.path}/state',
      );
      final protocol = WireProtocol(
        jsonDecode(File('assets/protocol-v5.schema.json').readAsStringSync())
            as Map<String, dynamic>,
      );
      final model = AppModel(CoreClient(paths, protocol));
      final server = model.remote;
      final http = HttpClient();
      addTearDown(() async {
        http.close(force: true);
        await server.stop();
        await model.tasks.shutdown();
        model.dispose();
        await temp.delete(recursive: true);
      });
      await server.start('127.0.0.1', port: 0);
      Future<({int status, dynamic body})> request(
        String method,
        String path, [
        Object? body,
      ]) async {
        final req = await http.openUrl(method, Uri.parse('${server.url}$path'));
        req.headers.set(
          HttpHeaders.authorizationHeader,
          'Bearer ${server.accessKey}',
        );
        if (method != 'GET') req.headers.set('Origin', server.url!);
        if (body != null) {
          req.headers.contentType = ContentType.json;
          req.write(jsonEncode(body));
        }
        final response = await req.close();
        final text = await utf8.decoder.bind(response).join();
        return (
          status: response.statusCode,
          body: response.headers.contentType?.mimeType == 'application/json'
              ? jsonDecode(text)
              : text,
        );
      }

      for (final path in [
        '/',
        '/app.js',
        '/style.css',
        '/tabler.min.css',
        '/LICENSE-tabler.txt',
      ]) {
        final asset = await request('GET', path);
        expect(asset.status, 200, reason: path);
        final filename = path == '/' ? 'index.html' : path.substring(1);
        expect(asset.body, File('../remote-web/$filename').readAsStringSync());
      }
      const code =
          'return { meta = { name = "远程来源", role = "fallback_indexer", pages = {"https://example.com/"} }, discover = function() error("save must not execute discover") end }';
      final imported = await request('POST', '/api/sources', {'code': code});
      expect(imported.status, 200);
      final id = imported.body['rule']['id'] as String;
      expect(imported.body['task_id'], isNull);
      expect(
        (await request('GET', '/api/sources/$id/script')).body['code'],
        code,
      );
      expect(
        (await request('GET', '/api/sources')).body['sources'],
        contains(containsPair('id', id)),
      );

      await Future.wait([
        model.mutate('set_source_enabled', {'source_id': id, 'enabled': false}),
        request('PUT', '/api/sources/$id/name', {'name': '远端改名'}),
      ]);
      final reopened = CoreClient(paths, protocol);
      final persisted = (await reopened.call(
        'list_sources',
      ) as List).singleWhere((source) => source['id'] == id) as Map;
      expect(persisted['name'], '远端改名');
      expect(persisted['enabled'], false);
      const changed = '$code\n-- edited through HTTP';
      expect(
        (await request('PUT', '/api/sources/$id/script', {
          'code': changed,
        })).status,
        200,
      );
      expect(
        (await reopened.call('get_script_source', {'source_id': id})
            as Map)['code'],
        changed,
      );
      final readonly = await request('PUT', '/api/sources/patchmoe/script', {
        'code': code,
      });
      expect(readonly.status, 422);
      expect(readonly.body['code'], 'source_builtin_readonly');
      final invalid = await request('PUT', '/api/sources/$id/script', {
        'code': 'not valid Lua',
      });
      expect(invalid.status, 422);
      expect(
        (await reopened.call('get_script_source', {'source_id': id})
            as Map)['code'],
        changed,
      );
      expect((await request('DELETE', '/api/sources/$id')).status, 200);
      expect(
        (await reopened.call('list_sources') as List).where(
          (source) => source['id'] == id,
        ),
        isEmpty,
      );
    },
    skip: binary.existsSync()
        ? false
        : 'Build the Rust CLI before this local HTTP/core check',
  );
}
