import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/services.dart';
import 'package:webview_flutter_windows/webview_flutter_windows.dart';

import 'protocol_models.dart';

/// Execute the site's normal JavaScript in an app-owned persistent session.
/// This controller is never mounted in a user-facing web page.
Future<List<Map<String, dynamic>>> collectSourceInBackground(
  String stateDirectory,
  String sourceUrl,
  Future<void> cancelled,
  void Function(String) progress,
) async {
  final controller = WebviewController();
  final subscriptions = <StreamSubscription<dynamic>>[];
  final response = Completer<Map<String, dynamic>>();
  var closed = false, collecting = false;
  var url = 'about:blank';
  void fail(String code, [String? detail]) {
    if (!closed && !response.isCompleted) {
      response.complete({'error': code, 'detail': detail});
    }
  }

  Future<void> collect() async {
    if (closed || collecting || url == 'about:blank') return;
    collecting = true;
    try {
      final script = await rootBundle.loadString('assets/shiravune-browser.js');
      if (!closed) {
        await controller.executeScript(
          '($script).then(value => window.chrome.webview.postMessage(JSON.stringify(value)))',
        );
      }
    } catch (error) {
      fail('source_browser_failed', '$error');
    }
  }

  Future<List<Map<String, dynamic>>> start() async {
    final profile = Directory('$stateDirectory/source-browser');
    await profile.create(recursive: true);
    await WebviewController.initializeEnvironment(userDataPath: profile.path);
    await controller.initialize();
    if (closed) throw const CoreError('task_cancelled');
    await controller.setSize(const Size(1280, 800));
    subscriptions.add(
      controller.url.listen((value) {
        if (closed || value == 'about:blank') return;
        final target = Uri.tryParse(value);
        if (target?.scheme != 'https' ||
            target?.host != 'www.patch.moe' ||
            target?.port != 443) {
          fail('source_browser_origin_changed');
        } else {
          url = value;
        }
      }),
    );
    subscriptions.add(
      controller.onLoadError.listen(
        (value) => fail('source_browser_load_failed', '$value'),
      ),
    );
    subscriptions.add(
      controller.loadingState.listen((value) {
        if (value == LoadingState.loading) collecting = false;
        if (value == LoadingState.navigationCompleted) unawaited(collect());
      }),
    );
    subscriptions.add(
      controller.webMessage.listen((value) {
        if (closed || response.isCompleted) return;
        collecting = false;
        try {
          final data = Map<String, dynamic>.from(
            (value is String ? jsonDecode(value) : value) as Map,
          );
          if (data['waiting'] == true) {
            progress('正在等待站点请求验证');
          } else {
            response.complete(data);
          }
        } catch (error) {
          fail('invalid_browser_response', '$error');
        }
      }, onError: (Object error) => fail('invalid_browser_response', '$error')),
    );
    progress('正在加载来源网页');
    await controller.setPopupWindowPolicy(WebviewPopupWindowPolicy.deny);
    await controller.loadUrl(sourceUrl);
    final result = await response.future;
    if (result['error'] != null) {
      throw CoreError(result['error'] as String, result['detail'] as String?);
    }
    return (result['pages'] as List)
        .map((page) => Map<String, dynamic>.from(page as Map))
        .toList();
  }

  try {
    return await Future.any([
      start(),
      cancelled.then<List<Map<String, dynamic>>>(
        (_) => throw const CoreError('task_cancelled'),
      ),
    ]);
  } finally {
    closed = true;
    for (final subscription in subscriptions) {
      await subscription.cancel();
    }
    await controller.dispose();
  }
}
