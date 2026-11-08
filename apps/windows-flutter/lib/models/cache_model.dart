import 'package:flutter/foundation.dart';

import '../host/core_client.dart';

/// App-owned state: navigation never cancels cleanup or discards its result.
class CacheModel extends ChangeNotifier {
  CacheModel(this.client);
  final CoreClient client;
  Map<String, dynamic>? inventory, result;
  Object? inventoryError, cleanupError;
  bool loading = false, cleaning = false;

  Future<void> refresh() async {
    if (loading || cleaning) return;
    loading = true;
    inventoryError = null;
    notifyListeners();
    try {
      inventory = await client.call('cache_status') as Map<String, dynamic>;
    } catch (error) {
      inventoryError = error;
    } finally {
      loading = false;
      notifyListeners();
    }
  }

  Future<void> cleanup() async {
    if (cleaning || loading) return;
    cleaning = true;
    cleanupError = null;
    result = null;
    notifyListeners();
    try {
      result = await client.call('cleanup_cache') as Map<String, dynamic>;
    } catch (error) {
      cleanupError = error;
    } finally {
      cleaning = false;
      // Refresh errors remain separate from the actual cleanup outcome.
      await refresh();
    }
  }
}
