import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:ui' show Size;

import 'package:webview_flutter_windows/webview_flutter_windows.dart';

import 'protocol_models.dart';

abstract interface class SourceBrowserSession {
  Future<Map<String, dynamic>> request(Map<String, dynamic> request);
  Future<void> close();
}

class WebViewSourceBrowser implements SourceBrowserSession {
  WebViewSourceBrowser(this.stateDirectory, this.cancelled);

  final String stateDirectory;
  final Future<void> cancelled;
  final _controller = WebviewController();
  late final Future<void> _ready = _initialize();
  bool _started = false;

  Future<void> _initialize() async {
    _started = true;
    final profile = await Directory('$stateDirectory/source-browser')
        .create(recursive: true);
    await WebviewController.initializeEnvironment(userDataPath: profile.path);
    await _controller.initialize();
    await _controller.setSize(const Size(1280, 800));
    await _controller.setPopupWindowPolicy(WebviewPopupWindowPolicy.deny);
  }

  Uri _requireUrl(String value, List<String> hosts) {
    final url = Uri.tryParse(value);
    if (url == null ||
        url.scheme != 'https' ||
        url.userInfo.isNotEmpty ||
        url.port != 443 ||
        !hosts.contains(url.host)) {
      throw const CoreError('source_browser_origin_changed');
    }
    return url;
  }

  @override
  Future<Map<String, dynamic>> request(Map<String, dynamic> request) async {
    final url = request['url'] as String;
    final method = request['method'] as String;
    final hosts = (request['page_hosts'] as List).cast<String>();
    final parsed = _requireUrl(url, hosts);
    final cancellation = cancelled.then<Never>(
      (_) => throw const CoreError('task_cancelled'),
    );
    await Future.any([_ready, cancellation]);
    final result = Completer<Map<String, dynamic>>();
    final requestToken = DateTime.now().microsecondsSinceEpoch.toString();
    final token = jsonEncode(requestToken);

    Future<void> execute(String action) async {
      try {
        await _controller.executeScript('''
          (async () => {
            try {
              const value = await ($action);
              window.chrome.webview.postMessage({token: $token, value});
            } catch (_) {
              window.chrome.webview.postMessage({token: $token, error: true});
            }
          })()
        ''');
      } catch (failure) {
        if (!result.isCompleted) result.completeError(failure);
      }
    }

    final messages = _controller.webMessage.listen((value) {
      if (result.isCompleted ||
          value is! Map ||
          value['token'] != requestToken) {
        return;
      }
      if (value['error'] == true) {
        result.completeError(const CoreError('source_browser_load_failed'));
      } else if (value['value'] case final Map payload) {
        result.complete(Map<String, dynamic>.from(payload));
      }
    });
    final origins = _controller.url.listen((value) {
      if (value == 'about:blank' || result.isCompleted) return;
      try {
        _requireUrl(value, hosts);
      } catch (failure) {
        result.completeError(failure);
        unawaited(_controller.stop());
      }
    });
    StreamSubscription<dynamic>? loading;
    Object? navigationError;
    final errors = _controller.onLoadError.listen((error) {
      navigationError = error;
    });
    final snapshot =
        '''
            (() => {
              if (!${jsonEncode(hosts)}.includes(location.hostname) ||
                  location.protocol !== 'https:' || location.port) throw Error('origin');
              if (document.readyState !== 'complete') return null;
              const snapshot = document.documentElement.cloneNode(true);
              const live = document.querySelectorAll('input');
              snapshot.querySelectorAll('input').forEach((input, index) => {
                input.setAttribute('value', live[index].value);
              });
              const navigation = performance.getEntriesByType('navigation')[0];
              const status = navigation?.responseStatus;
              return {response_url: location.href, status, text: snapshot.outerHTML};
            })()
          ''';
    try {
      if (method == 'GET' || method == 'SNAPSHOT') {
        loading = _controller.loadingState.listen((state) {
          if (state != LoadingState.navigationCompleted || result.isCompleted) {
            return;
          }
          unawaited(execute(snapshot));
        });
        if (method == 'GET') {
          await _controller.loadUrl(url);
        } else {
          unawaited(execute(snapshot));
        }
      } else if (method == 'POST') {
        unawaited(
          execute('''
          (async () => {
            const destination = new URL(${jsonEncode(parsed.toString())});
            if (destination.origin !== location.origin) throw Error('origin');
            const response = await fetch(destination, {
              method: 'POST',
              body: ${jsonEncode(request['body'])},
              headers: {'Content-Type': ${jsonEncode(request['content_type'])}},
              credentials: 'same-origin',
              mode: 'same-origin',
              redirect: 'error',
            });
            return {response_url: response.url, status: response.status, text: await response.text()};
          })()
        '''),
        );
      } else {
        throw const CoreError('invalid_browser_request');
      }
      final data = await Future.any([result.future, cancellation]);
      _requireUrl(data['response_url'] as String, hosts);
      final status = data['status'];
      if (status is! int ||
          status < 100 ||
          status > 599 ||
          (navigationError != null && status < 400)) {
        throw CoreError(
          'source_browser_load_failed',
          navigationError?.toString(),
        );
      }
      return {...data, 'url': url, 'method': method, 'body': request['body']};
    } finally {
      await origins.cancel();
      await messages.cancel();
      await errors.cancel();
      await loading?.cancel();
    }
  }

  @override
  Future<void> close() async {
    if (_started) await _controller.dispose();
  }
}
