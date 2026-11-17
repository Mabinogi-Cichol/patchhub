import 'package:flutter/foundation.dart';

import '../host/core_client.dart';

class CacheBucket {
  CacheBucket({
    required this.statusMethod,
    required this.cleanupMethod,
    this.onCleaned,
  });

  final String statusMethod;
  final String cleanupMethod;
  final VoidCallback? onCleaned;

  Map<String, dynamic>? inventory, result;
  Object? inventoryError, cleanupError;
  bool loading = false, cleaning = false;
}

/// App-owned state: navigation never cancels cleanup or discards its result.
class CacheModel extends ChangeNotifier {
  CacheModel(this.client, {VoidCallback? onGameCacheCleaned})
    : download = CacheBucket(
        statusMethod: 'cache_status',
        cleanupMethod: 'cleanup_cache',
      ),
      game = CacheBucket(
        statusMethod: 'game_cache_status',
        cleanupMethod: 'cleanup_game_cache',
        onCleaned: onGameCacheCleaned,
      );

  final CoreClient client;
  final CacheBucket download;
  final CacheBucket game;

  Map<String, dynamic>? get inventory => download.inventory;
  Map<String, dynamic>? get result => download.result;
  Object? get inventoryError => download.inventoryError;
  Object? get cleanupError => download.cleanupError;
  bool get loading => download.loading;
  bool get cleaning => download.cleaning;

  Future<void> refresh() => _refresh(download);
  Future<void> cleanup() => _cleanup(download);
  Future<void> refreshGame() => _refresh(game);
  Future<void> cleanupGame() => _cleanup(game);

  Future<void> _refresh(CacheBucket bucket) async {
    if (bucket.loading || bucket.cleaning) return;
    bucket.loading = true;
    bucket.inventoryError = null;
    notifyListeners();
    try {
      bucket.inventory =
          await client.call(bucket.statusMethod) as Map<String, dynamic>;
    } catch (error) {
      bucket.inventoryError = error;
    } finally {
      bucket.loading = false;
      notifyListeners();
    }
  }

  Future<void> _cleanup(CacheBucket bucket) async {
    if (bucket.cleaning || bucket.loading) return;
    bucket.cleaning = true;
    bucket.cleanupError = null;
    bucket.result = null;
    notifyListeners();
    try {
      bucket.result =
          await client.call(bucket.cleanupMethod) as Map<String, dynamic>;
      bucket.onCleaned?.call();
    } catch (error) {
      bucket.cleanupError = error;
    } finally {
      bucket.cleaning = false;
      // Refresh errors remain separate from the actual cleanup outcome.
      await _refresh(bucket);
    }
  }
}
