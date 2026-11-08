import 'dart:async';

import 'package:flutter/material.dart';

import '../models/app_model.dart';
import 'app_update_panel.dart';
import 'cache_page.dart';
import 'source_pages.dart';

class SettingsPage extends StatefulWidget {
  const SettingsPage({super.key, required this.model});
  final AppModel model;
  @override
  State<SettingsPage> createState() => _SettingsPageState();
}

class _SettingsPageState extends State<SettingsPage> {
  int selected = 0;
  bool cacheVisited = false;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Row(
      children: [
        SizedBox(
          width: 232,
          child: Material(
            color: scheme.surface,
            child: ListView(
              padding: const EdgeInsets.all(12),
              children: [
                Padding(
                  padding: const EdgeInsets.fromLTRB(12, 16, 12, 24),
                  child: Text(
                    '设置',
                    style: Theme.of(context).textTheme.titleLarge,
                  ),
                ),
                for (final (index, label, icon) in const [
                  (0, '全局来源', Icons.folder_open_outlined),
                  (1, '缓存管理', Icons.storage_outlined),
                  (2, '关于', Icons.info_outline),
                ])
                  Padding(
                    padding: const EdgeInsets.only(bottom: 8),
                    child: ListTile(
                      selected: selected == index,
                      selectedTileColor: scheme.secondary.withValues(
                        alpha: 0.16,
                      ),
                      selectedColor: scheme.secondary,
                      shape: RoundedRectangleBorder(
                        borderRadius: BorderRadius.circular(12),
                      ),
                      leading: Icon(icon, size: 20),
                      title: Text(label),
                      onTap: () => setState(() {
                        selected = index;
                        if (index == 1) cacheVisited = true;
                      }),
                    ),
                  ),
              ],
            ),
          ),
        ),
        const VerticalDivider(width: 1),
        Expanded(
          child: IndexedStack(
            index: selected,
            children: [
              GlobalSourcesPage(model: widget.model),
              cacheVisited
                  ? CachePage(model: widget.model.cache)
                  : const SizedBox.shrink(),
              const AboutPage(),
            ],
          ),
        ),
      ],
    );
  }
}

class AboutPage extends StatelessWidget {
  const AboutPage({super.key});

  @override
  Widget build(BuildContext context) {
    final scope = AppUpdateScope.maybeOf(context);
    if (scope == null) return const Center(child: Text('更新服务未初始化'));
    final u = scope.update;
    return ListenableBuilder(
      listenable: u,
      builder: (context, _) {
        final state = u.result['state'];
        final status = switch (state) {
          'checking' => '正在检查更新…',
          'up_to_date' => '当前已是最新正式版',
          'available' =>
            '有新版本 ${u.result['tag']} 可用${u.skipped ? '（已跳过）' : ''}',
          'downloading' => u.downloadLabel,
          'ready' => '更新已就绪 ${u.result['tag']}',
          'failed' || 'unavailable' => '更新操作失败：${u.result['error']}',
          _ => '尚未检查更新',
        };
        return ListView(
          key: const PageStorageKey('settings-about'),
          padding: const EdgeInsets.all(28),
          children: [
            Text('关于', style: Theme.of(context).textTheme.headlineSmall),
            const SizedBox(height: 8),
            const Text('应用信息与版本更新'),
            const SizedBox(height: 32),
            Text('PatchHub', style: Theme.of(context).textTheme.headlineMedium),
            const SizedBox(height: 8),
            const Text('为 Steam 游戏查找与安装补丁'),
            const SizedBox(height: 24),
            Card(
              child: Padding(
                padding: const EdgeInsets.all(24),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Wrap(
                      spacing: 24,
                      runSpacing: 16,
                      crossAxisAlignment: WrapCrossAlignment.center,
                      children: [
                        Text(
                          '当前版本 ${u.current}',
                          style: Theme.of(context).textTheme.titleLarge,
                        ),
                        OutlinedButton.icon(
                          onPressed: u.busy || state == 'ready'
                              ? null
                              : () => unawaited(u.check(manual: true)),
                          icon: const Icon(Icons.refresh, size: 18),
                          label: Text(state == 'checking' ? '正在检查…' : '检查更新'),
                        ),
                      ],
                    ),
                    const Divider(height: 32),
                    Semantics(liveRegion: true, child: SelectableText(status)),
                    const SizedBox(height: 8),
                    Text('上次成功检查：${sourceTime(u.data['last_success_at'])}'),
                    if (u.hasUpdate || u.outcome != null)
                      TextButton(
                        onPressed: scope.showDetails,
                        child: const Text('查看更新'),
                      ),
                  ],
                ),
              ),
            ),
          ],
        );
      },
    );
  }
}
