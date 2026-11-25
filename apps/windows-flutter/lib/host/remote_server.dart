import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import '../models/app_model.dart';
import 'core_client.dart';
import 'core_task.dart';

class RemoteServer extends ChangeNotifier {
  static const preferredPort = 47625;
  RemoteServer(this.model, {Future<Uint8List> Function(String)? loadAsset})
    : _loadAsset = loadAsset ?? _bundleAsset;

  final AppModel model;
  final Future<Uint8List> Function(String) _loadAsset;
  HttpServer? _server;
  Future<void>? _starting;
  final _tasks = <String, CoreTask>{};
  bool _disposed = false;
  bool changing = false;
  Object? error;
  String? url;
  String? accessKey;
  int? _lastPin;
  bool get enabled => _server != null;

  static Future<Uint8List> _bundleAsset(String name) async =>
      (await rootBundle.load('packages/patchhub_remote_web/$name')).buffer
          .asUint8List();

  static Future<List<({String name, String address})>> addresses() async {
    final interfaces = await NetworkInterface.list(
      type: InternetAddressType.IPv4,
    );
    final physical = (await const MethodChannel('patchhub/desktop')
        .invokeListMethod<int>('physicalLanInterfaces'))!;
    return [
      for (final preferred in [true, false])
        for (final interface in interfaces)
          if (physical.contains(interface.index) == preferred)
            for (final address in interface.addresses)
              (name: interface.name, address: address.address),
    ];
  }

  void _changed() {
    if (!_disposed) notifyListeners();
  }

  Future<void> start(String address, {int? port}) async {
    if (changing || enabled) throw const CoreError('remote_busy');
    changing = true;
    error = null;
    _changed();
    final operation = _bind(address, port);
    _starting = operation;
    try {
      await operation;
    } catch (failure) {
      error = failure;
      rethrow;
    } finally {
      _starting = null;
      changing = false;
      _changed();
    }
  }

  Future<void> _bind(String address, int? port) async {
    final ip = InternetAddress(address);
    if (ip.type != InternetAddressType.IPv4 ||
        ip.isMulticast ||
        ip.address == '0.0.0.0') {
      throw const CoreError('invalid_address', '请选择本机 IPv4 地址');
    }
    HttpServer server;
    try {
      server = await HttpServer.bind(ip, port ?? preferredPort);
    } on SocketException catch (failure) {
      // Winsock reports WSAEACCES when another socket owns the port exclusively.
      final unavailablePort = Platform.isWindows
          ? const [10048, 10013]
          : const [98];
      if (port != null ||
          !unavailablePort.contains(failure.osError?.errorCode)) {
        rethrow;
      }
      server = await HttpServer.bind(ip, 0);
    }
    _server = server;
    url = 'http://${ip.address}:${server.port}';
    final random = Random.secure();
    int pin;
    do {
      pin = random.nextInt(1000000);
    } while (pin == _lastPin);
    _lastPin = pin;
    accessKey = pin.toString().padLeft(6, '0');
    server.listen((request) => unawaited(_handle(request)));
  }

  Future<void> stop() async {
    final starting = _starting;
    if (starting != null) {
      // A failed bind still leaves nothing to stop. A successful bind must be
      // closed even when shutdown races with the settings switch.
      try {
        await starting;
      } on SocketException catch (_) {
        // The bind failure is already exposed by start().
      } on CoreError catch (_) {
        // Invalid local addresses are likewise reported by start().
      }
    }
    final server = _server;
    _server = null;
    url = null;
    accessKey = null;
    _tasks.clear();
    _changed();
    await server?.close(force: true);
  }

  static const _assets = {
    '/': ('index.html', 'text/html; charset=utf-8'),
    '/index.html': ('index.html', 'text/html; charset=utf-8'),
    '/app.js': ('app.js', 'application/javascript; charset=utf-8'),
    '/tabler.min.css': ('tabler.min.css', 'text/css; charset=utf-8'),
    '/style.css': ('style.css', 'text/css; charset=utf-8'),
    '/LICENSE-tabler.txt': ('LICENSE-tabler.txt', 'text/plain; charset=utf-8'),
  };

  Future<void> _handle(HttpRequest request) async {
    final response = request.response;
    response.headers
      ..set('X-Content-Type-Options', 'nosniff')
      ..set('Cache-Control', 'no-store')
      ..set('Referrer-Policy', 'no-referrer')
      ..set(
        'Content-Security-Policy',
        "default-src 'self'; script-src 'self'; style-src 'self'; "
            "img-src 'self' data:; font-src 'self' data:; "
            "connect-src 'self'; frame-ancestors 'none'; base-uri 'none'",
      );
    try {
      final currentUrl = url;
      if (currentUrl == null ||
          request.headers.value(HttpHeaders.hostHeader) !=
              Uri.parse(currentUrl).authority) {
        throw const _HttpError(403, 'invalid_host');
      }
      final path = request.uri.path;
      if (!path.startsWith('/api/')) {
        final asset = _assets[path];
        if (request.method != 'GET' || asset == null) {
          throw const _HttpError(404, 'not_found');
        }
        response.headers.set(HttpHeaders.contentTypeHeader, asset.$2);
        response.add(await _loadAsset(asset.$1));
      } else {
        if (request.headers.value(HttpHeaders.authorizationHeader) !=
            'Bearer $accessKey') {
          throw const _HttpError(401, 'unauthorized');
        }
        if (request.method != 'GET' &&
            request.headers.value('Origin') != currentUrl) {
          throw const _HttpError(403, 'invalid_origin');
        }
        if (model.tasks.closing) throw const CoreError('app_closing');
        final value = await _api(request);
        response.headers.contentType = ContentType.json;
        response.write(jsonEncode(value));
      }
    } on _HttpError catch (failure) {
      _writeError(response, failure.status, failure.code, failure.detail);
    } on CoreError catch (failure) {
      final busy =
          failure.code.endsWith('_busy') || failure.code == 'app_closing';
      _writeError(response, busy ? 409 : 422, failure.code, failure.detail);
    } on FormatException catch (failure) {
      _writeError(response, 400, 'invalid_request', failure.message);
    } catch (failure, stack) {
      debugPrint('Remote 请求失败：$failure\n$stack');
      _writeError(response, 500, 'remote_failed', '$failure');
    } finally {
      try {
        await response.close();
      } on SocketException catch (_) {
        // A browser can disconnect while the host operation completes.
      } on HttpException catch (_) {
        // Stopping Remote also closes active HTTP connections.
      }
    }
  }

  void _writeError(
    HttpResponse response,
    int status,
    String code,
    String? detail,
  ) {
    response.statusCode = status;
    response.headers.contentType = ContentType.json;
    response.write(jsonEncode({'code': code, 'detail': detail}));
  }

  Future<Map<String, dynamic>> _body(
    HttpRequest request, {
    bool allowEmpty = false,
  }) async {
    final text = await utf8.decoder.bind(request).join();
    if (allowEmpty && text.isEmpty) return {};
    final value = jsonDecode(text);
    if (value is! Map<String, dynamic>) {
      throw const _HttpError(400, 'invalid_request', '需要 JSON 对象');
    }
    return value;
  }

  T _field<T>(Map<String, dynamic> body, String key) {
    final value = body[key];
    if (value is! T) throw _HttpError(400, 'invalid_request', '无效字段：$key');
    return value;
  }

  CoreTask _update(String id, bool force) {
    final source = model.sources
        .where((source) => source['id'] == id)
        .firstOrNull;
    if (source == null) throw const _HttpError(404, 'source_not_found');
    if (source['role'] != 'publisher') {
      throw const _HttpError(422, 'invalid_source_role');
    }
    final task = model.startSource('update_source', {
      'source_id': id,
      'force': force,
    });
    _rememberTask(task);
    return task;
  }

  void _rememberTask(CoreTask task) {
    _tasks.removeWhere(
      (_, previous) =>
          !previous.running &&
          previous.arguments['source_id'] == task.arguments['source_id'],
    );
    _tasks[task.requestId] = task;
  }

  Map<String, dynamic>? _error(CoreError? error) =>
      error == null ? null : {'code': error.code, 'detail': error.detail};

  Map<String, dynamic> _taskSnapshot(CoreTask task) => {
    'task_id': task.requestId,
    'method': task.method,
    'running': task.running,
    'cancel_requested': task.cancelRequested,
    'progress': task.progress,
    'result': task.result,
    'error': _error(task.error),
    'control_error': _error(task.controlError),
    'diagnostics': task.diagnostics,
  };

  Future<Object?> _api(HttpRequest request) async {
    final parts = request.uri.pathSegments;
    final method = request.method;
    if (parts.length == 2) {
      switch ((method, parts[1])) {
        case ('GET', 'sources'):
          await model.loadSources();
          return {
            'sources': model.sources,
            'settings_revision': model.settingsRevision,
          };
        case ('GET', 'script-prompt'):
          return model.client.call('get_script_source_prompt');
        case ('POST', 'sources'):
          final body = await _body(request);
          final value = await model.mutate('import_script_source', {
            'api_version': 1,
            'code': _field<String>(body, 'code'),
          });
          final id = (value['rule'] as Map)['id'];
          final task = model.tasks.sources['update_source:$id'];
          if (task != null) {
            _rememberTask(task);
            return {...value, 'task_id': task.requestId};
          }
          return value;
        case ('POST', 'updates'):
          final body = await _body(request, allowEmpty: true);
          final force = body.containsKey('force')
              ? _field<bool>(body, 'force')
              : false;
          await model.loadSources();
          return {
            'task_ids': [
              for (final source in model.sources)
                if (source['enabled'] == true &&
                    source['role'] == 'publisher' &&
                    model
                            .tasks
                            .sources['update_source:${source['id']}']
                            ?.running !=
                        true)
                  _update(source['id'] as String, force).requestId,
            ],
          };
      }
    }
    if (parts.length >= 3 && parts[1] == 'sources') {
      final id = parts[2];
      if (parts.length == 3 && method == 'DELETE') {
        final result = await model.mutate('remove_source', {'source_id': id});
        _tasks.removeWhere(
          (_, task) => !task.running && task.arguments['source_id'] == id,
        );
        return result;
      }
      if (parts.length == 4) {
        final action = parts[3];
        if (method == 'GET' && action == 'script') {
          return model.client.call('get_script_source', {'source_id': id});
        }
        if (method == 'PUT') {
          final body = await _body(request);
          final (coreMethod, key, value) = switch (action) {
            'script' => (
              'import_script_source',
              'code',
              _field<String>(body, 'code'),
            ),
            'enabled' => (
              'set_source_enabled',
              'enabled',
              _field<bool>(body, 'enabled'),
            ),
            'name' => ('set_source_name', 'name', _field<String>(body, 'name')),
            'interval' => (
              'set_source_update_interval',
              'interval_hours',
              _field<int>(body, 'interval_hours'),
            ),
            _ => throw const _HttpError(404, 'not_found'),
          };
          return model.mutate(coreMethod, {
            'source_id': id,
            key: value,
            if (action == 'script') 'api_version': 1,
          });
        }
        if (method == 'POST' && action == 'updates') {
          final body = await _body(request, allowEmpty: true);
          final force = body.containsKey('force')
              ? _field<bool>(body, 'force')
              : false;
          await model.loadSources();
          return {'task_id': _update(id, force).requestId};
        }
      }
    }
    if (parts.length >= 3 && parts[1] == 'tasks') {
      final task = _tasks[parts[2]];
      if (task == null) throw const _HttpError(404, 'task_not_found');
      if (parts.length == 3 && method == 'GET') {
        return _taskSnapshot(task);
      }
      if (parts.length == 4 && parts[3] == 'cancel' && method == 'POST') {
        await task.cancel();
        return _taskSnapshot(task);
      }
    }
    throw const _HttpError(404, 'not_found');
  }

  @override
  void dispose() {
    _disposed = true;
    unawaited(stop());
    super.dispose();
  }
}

class _HttpError implements Exception {
  const _HttpError(this.status, this.code, [this.detail]);
  final int status;
  final String code;
  final String? detail;
}
