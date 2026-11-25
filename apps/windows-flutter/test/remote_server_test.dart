import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/host/core_client.dart';
import 'package:patchhub_windows/host/remote_server.dart';
import 'package:patchhub_windows/models/app_model.dart';

import 'support/process_fixture_client.dart';

class RemoteClient extends ProcessFixtureClient {
  RemoteClient([super.scenario = 'normal']);
  int revision = 1;
  int activeWrites = 0;
  int maxWrites = 0;
  final mutations = <String>[];
  final sources = <Map<String, dynamic>>[
    {'id': 'publisher', 'role': 'publisher', 'enabled': true, 'name': '索引'},
    {
      'id': 'fallback',
      'role': 'fallback_indexer',
      'enabled': true,
      'name': '补查',
    },
    {'id': 'disabled', 'role': 'publisher', 'enabled': false, 'name': '停用'},
  ];

  @override
  Future<Object?> call(
    String method, [
    Map<String, Object?> args = const {},
  ]) async {
    if (method == 'list_sources') {
      return sources.map((source) => {...source}).toList();
    }
    if (method == 'get_script_source') {
      return {
        'source_id': args['source_id'],
        'api_version': 1,
        'code': 'return {}',
      };
    }
    if (method == 'get_script_source_prompt') return {'prompt': '提示词'};
    mutations.add(method);
    activeWrites++;
    if (activeWrites > maxWrites) maxWrites = activeWrites;
    await Future<void>.delayed(const Duration(milliseconds: 5));
    try {
      if (args['code'] == 'bad') throw const CoreError('invalid_lua', 'Lua 错误');
      if (method == 'import_script_source') {
        return {
          'settings_revision': ++revision,
          'rule': {
            'id': args['source_id'] ?? 'imported',
            'role': 'fallback_indexer',
            'enabled': true,
          },
        };
      }
      if (method == 'remove_source') {
        sources.removeWhere((source) => source['id'] == args['source_id']);
      } else {
        final source = sources.firstWhere(
          (source) => source['id'] == args['source_id'],
        );
        for (final key in ['enabled', 'name', 'interval_hours']) {
          if (args.containsKey(key)) source[key] = args[key];
        }
      }
      return {'settings_revision': ++revision};
    } finally {
      activeWrites--;
    }
  }
}

void main() {
  late RemoteClient client;
  late AppModel model;
  late RemoteServer remote;
  late HttpClient http;

  Future<({int status, dynamic body})> request(
    String method,
    String path, {
    Object? body,
    String? key,
    bool authenticated = true,
    String? origin,
    bool includeOrigin = true,
    String? host,
  }) async {
    final req = await http.openUrl(method, Uri.parse('${remote.url}$path'));
    if (authenticated) {
      req.headers.set(
        HttpHeaders.authorizationHeader,
        'Bearer ${key ?? remote.accessKey}',
      );
    }
    if (includeOrigin && method != 'GET') {
      req.headers.set('Origin', origin ?? remote.url!);
    }
    if (host != null) req.headers.set(HttpHeaders.hostHeader, host);
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

  setUp(() async {
    client = RemoteClient();
    model = AppModel(client);
    remote = RemoteServer(
      model,
      loadAsset: (name) async => Uint8List.fromList(utf8.encode('asset:$name')),
    );
    http = HttpClient();
    await remote.start('127.0.0.1', port: 0);
  });

  tearDown(() async {
    http.close(force: true);
    await remote.stop();
    remote.dispose();
    await model.tasks.shutdown();
    await Future<void>.delayed(Duration.zero);
    model.dispose();
  });

  test('static pages contain no key; all API reads require the session key and exact host', () async {
    final page = await request('GET', '/', authenticated: false);
    expect(page.status, 200);
    expect(page.body, 'asset:index.html');
    expect(
      (await request('GET', '/api/sources', authenticated: false)).status,
      401,
    );
    expect(
      (await request('GET', '/api/script-prompt', key: 'wrong')).status,
      401,
    );
    expect(
      (await request('GET', '/api/sources', host: 'other.local')).status,
      403,
    );
    expect(
      (await request('GET', '/api/sources')).body['sources'],
      hasLength(3),
    );
    expect((await request('GET', '/api/script-prompt')).body['prompt'], '提示词');
    expect(
      (await request('GET', '/api/sources/fallback/script')).body['code'],
      'return {}',
    );
    expect((await request('GET', '/api/arbitrary-rpc')).status, 404);
    expect((await request('GET', '/not-a-bundled-file')).status, 404);
  });

  test(
    'mutations reject cross-origin and missing Origin before invoking core',
    () async {
      for (final origin in ['http://other.local', 'null']) {
        expect(
          (await request(
            'POST',
            '/api/sources',
            body: {'code': 'return {}'},
            origin: origin,
          )).status,
          403,
        );
      }
      expect(
        (await request(
          'POST',
          '/api/sources',
          body: {'code': 'return {}'},
          includeOrigin: false,
        )).status,
        403,
      );
      expect(client.mutations, isEmpty);
    },
  );

  test('source operations reuse model and preserve domain errors', () async {
    final imported = await request(
      'POST',
      '/api/sources',
      body: {'code': 'return {}'},
    );
    expect(imported.status, 200);
    expect(imported.body['rule']['id'], 'imported');
    expect(
      (await request(
        'PUT',
        '/api/sources/fallback/script',
        body: {'code': 'return {}'},
      )).body['rule']['id'],
      'fallback',
    );
    expect(
      (await request(
        'PUT',
        '/api/sources/fallback/name',
        body: {'name': '新名称'},
      )).status,
      200,
    );
    expect(
      (await request(
        'PUT',
        '/api/sources/fallback/enabled',
        body: {'enabled': false},
      )).status,
      200,
    );
    expect(
      (await request(
        'PUT',
        '/api/sources/publisher/interval',
        body: {'interval_hours': 12},
      )).status,
      200,
    );
    expect(
      model.sources.firstWhere((source) => source['id'] == 'fallback')['name'],
      '新名称',
    );
    expect(
      model.sources.firstWhere(
        (source) => source['id'] == 'fallback',
      )['enabled'],
      false,
    );
    final invalid = await request(
      'POST',
      '/api/sources',
      body: {'code': 'bad'},
    );
    expect(invalid.status, 422);
    expect(invalid.body, {'code': 'invalid_lua', 'detail': 'Lua 错误'});
    expect(
      (await request(
        'PUT',
        '/api/sources/fallback/enabled',
        body: {'enabled': 'yes'},
      )).status,
      400,
    );
    expect((await request('DELETE', '/api/sources/fallback')).status, 200);
    expect(
      model.sources.where((source) => source['id'] == 'fallback'),
      isEmpty,
    );
  });

  test(
    'local and remote writes share serialization and refresh revisions',
    () async {
      await Future.wait([
        model.mutate('set_source_enabled', {
          'source_id': 'fallback',
          'enabled': false,
        }),
        request('PUT', '/api/sources/publisher/name', body: {'name': '远程名称'}),
        request(
          'PUT',
          '/api/sources/disabled/interval',
          body: {'interval_hours': 7},
        ),
      ]);
      expect(client.maxWrites, 1);
      expect(model.settingsRevision, 4);
      expect(
        model.sources.firstWhere(
          (source) => source['id'] == 'publisher',
        )['name'],
        '远程名称',
      );
      expect(
        model.sources.firstWhere(
          (source) => source['id'] == 'fallback',
        )['enabled'],
        false,
      );
    },
  );

  test('update-all filters disabled and fallback sources, and exposes host task progress', () async {
    final result = await request('POST', '/api/updates', body: {'force': true});
    expect(result.status, 200);
    final ids = result.body['task_ids'] as List;
    expect(ids, hasLength(1));
    final task = model.tasks.sources['update_source:publisher']!;
    expect(ids.single, task.requestId);
    expect(task.arguments['force'], true);
    await task.done;
    expect(
      (await request('GET', '/api/tasks/${ids.single}')).body['running'],
      false,
    );
    final replacement = await request('POST', '/api/sources/publisher/updates');
    expect(replacement.status, 200);
    expect((await request('GET', '/api/tasks/${ids.single}')).status, 404);
    await model.tasks.sources['update_source:publisher']!.done;
    expect((await request('GET', '/api/tasks/not-a-remote-task')).status, 404);
    expect(
      (await request('POST', '/api/sources/fallback/updates', body: {})).status,
      422,
    );
  });

  test('cancellation uses actual task handle and task receipt', () async {
    await remote.stop();
    remote.dispose();
    model.dispose();
    client = RemoteClient('cancel_wait');
    model = AppModel(client);
    remote = RemoteServer(model);
    await remote.start('127.0.0.1', port: 0);
    final result = await request(
      'POST',
      '/api/sources/publisher/updates',
      body: {},
    );
    final id = result.body['task_id'];
    expect((await request('GET', '/api/tasks/$id')).body['running'], true);
    expect(
      (await request('POST', '/api/tasks/$id/cancel')).body['cancel_requested'],
      true,
    );
    await model.tasks.sources['update_source:publisher']!.done;
    final receipt = await request('GET', '/api/tasks/$id');
    expect(receipt.body['running'], false);
    expect(receipt.body['error']['code'], 'task_cancelled');
  });

  test(
    'stopping Remote forgets handles and leaves the host task running',
    () async {
      await remote.stop();
      remote.dispose();
      model.dispose();
      client = RemoteClient('cancel_wait');
      model = AppModel(client);
      remote = RemoteServer(model);
      await remote.start('127.0.0.1', port: 0);
      final result = await request('POST', '/api/sources/publisher/updates');
      final task = model.tasks.sources['update_source:publisher']!;
      expect((await request('DELETE', '/api/sources/publisher')).status, 409);
      await remote.stop();
      expect(task.running, true);
      expect(task.cancelRequested, false);
      await remote.start('127.0.0.1', port: 0);
      expect(
        (await request('GET', '/api/tasks/${result.body['task_id']}')).status,
        404,
      );
      await task.cancel();
      await task.done;
    },
  );

  test(
    'stopping closes the socket and re-enabling invalidates the previous key',
    () async {
      final previousUrl = remote.url!;
      final previousKey = remote.accessKey!;
      expect(previousKey, matches(RegExp(r'^[0-9]{6}$')));
      await remote.stop();
      expect(remote.enabled, false);
      expect(remote.url, isNull);
      expect(remote.accessKey, isNull);
      await expectLater(
        http.getUrl(Uri.parse(previousUrl)),
        throwsA(isA<SocketException>()),
      );
      await remote.start('127.0.0.1', port: 0);
      expect(remote.accessKey, isNot(previousKey));
      expect(
        (await request('GET', '/api/sources', key: previousKey)).status,
        401,
      );
    },
  );

  test('default port stays fixed and falls back only while occupied', () async {
    const address = '127.0.0.2';
    await remote.stop();
    await remote.start(address);
    expect(Uri.parse(remote.url!).port, RemoteServer.preferredPort);
    await remote.stop();
    final occupied = await ServerSocket.bind(
      InternetAddress.anyIPv4,
      RemoteServer.preferredPort,
    );
    try {
      await remote.start(address);
      expect(Uri.parse(remote.url!).port, isNot(RemoteServer.preferredPort));
      await remote.stop();
    } finally {
      await occupied.close();
    }
    await remote.start(address);
    expect(Uri.parse(remote.url!).port, RemoteServer.preferredPort);
  });

  test('bind failure never reports a running URL or key', () async {
    await remote.stop();
    final occupied = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    try {
      await expectLater(
        remote.start('127.0.0.1', port: occupied.port),
        throwsA(isA<SocketException>()),
      );
      expect(remote.enabled, false);
      expect(remote.error, isA<SocketException>());
      expect(remote.url, isNull);
      expect(remote.accessKey, isNull);
    } finally {
      await occupied.close(force: true);
    }
  });
}
