import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:url_launcher/url_launcher.dart';

import '../host/remote_server.dart';

class RemotePage extends StatefulWidget {
  const RemotePage({super.key, required this.server});
  final RemoteServer server;

  @override
  State<RemotePage> createState() => _RemotePageState();
}

class _RemotePageState extends State<RemotePage> {
  List<({String name, String address})>? addresses;
  String? selected;
  Object? addressError;

  @override
  void initState() {
    super.initState();
    loadAddresses();
  }

  Future<void> loadAddresses() async {
    try {
      final result = await RemoteServer.addresses();
      if (!mounted) return;
      setState(() {
        addresses = result;
        final listening = widget.server.url == null
            ? null
            : Uri.parse(widget.server.url!).host;
        selected = result.any((address) => address.address == listening)
            ? listening
            : result.firstOrNull?.address;
        addressError = null;
      });
    } catch (failure) {
      if (mounted) setState(() => addressError = failure);
    }
  }

  Future<void> toggle(bool enabled) async {
    try {
      if (enabled) {
        await widget.server.start(selected!);
      } else {
        await widget.server.stop();
      }
    } catch (failure) {
      if (!mounted) return;
      ScaffoldMessenger.of(context)
          .showSnackBar(SnackBar(content: Text('Remote 操作失败：$failure')));
    }
  }

  void showMessage(String message) {
    if (!mounted) return;
    ScaffoldMessenger.of(context)
        .showSnackBar(SnackBar(content: Text(message)));
  }

  Future<void> openRemote(String url) async {
    try {
      if (!await launchUrl(
        Uri.parse(url),
        mode: LaunchMode.externalApplication,
      )) {
        showMessage('无法打开 Remote 网页');
      }
    } catch (failure) {
      showMessage('无法打开 Remote 网页：$failure');
    }
  }

  Future<void> copyKey(String key) async {
    try {
      await Clipboard.setData(ClipboardData(text: key));
      showMessage('已复制密钥');
    } catch (failure) {
      showMessage('复制密钥失败：$failure');
    }
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.server,
    builder: (context, _) {
      final server = widget.server;
      final theme = Theme.of(context);
      return ListView(
        key: const PageStorageKey('settings-remote'),
        padding: const EdgeInsets.all(28),
        children: [
          Text('Remote', style: Theme.of(context).textTheme.headlineSmall),
          const SizedBox(height: 8),
          const Text('在同一局域网的浏览器中管理这台设备的全局来源及导入 Lua。'),
          const SizedBox(height: 24),
          if (addresses != null && addresses!.isNotEmpty)
            DropdownButtonFormField<String>(
              initialValue: selected,
              isExpanded: true,
              decoration: const InputDecoration(labelText: '局域网 IPv4 地址'),
              items: [
                for (final address in addresses!)
                  DropdownMenuItem(
                    value: address.address,
                    child: Text(
                      '${address.address} · ${address.name}',
                      overflow: TextOverflow.ellipsis,
                    ),
                  ),
              ],
              onChanged: server.enabled || server.changing
                  ? null
                  : (value) => setState(() => selected = value),
            ),
          if (addresses?.isEmpty == true) const Text('未找到可用的局域网 IPv4 地址。'),
          if (addressError != null) SelectableText('地址读取失败：$addressError'),
          if (!server.enabled)
            Align(
              alignment: Alignment.centerLeft,
              child: TextButton.icon(
                onPressed: server.changing ? null : loadAddresses,
                icon: const Icon(Icons.refresh),
                label: const Text('刷新地址'),
              ),
            ),
          SwitchListTile(
            contentPadding: EdgeInsets.zero,
            title: const Text('启用 Remote'),
            subtitle: Text(
              server.changing
                  ? '正在启动…'
                  : server.enabled
                  ? '正在监听，离开此页后仍可访问'
                  : '已关闭',
            ),
            value: server.enabled,
            onChanged: server.changing || (!server.enabled && selected == null)
                ? null
                : toggle,
          ),
          if (server.error != null) SelectableText('启动失败：${server.error}'),
          if (server.enabled) ...[
            const SizedBox(height: 12),
            Card(
              margin: EdgeInsets.zero,
              color: theme.colorScheme.surfaceContainerHigh,
              child: Padding(
                padding: const EdgeInsets.all(20),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text('访问地址', style: theme.textTheme.labelLarge),
                    const SizedBox(height: 4),
                    SelectableText(
                      server.url!,
                      style: theme.textTheme.titleMedium,
                    ),
                    const SizedBox(height: 16),
                    Text('本次访问密钥', style: theme.textTheme.labelLarge),
                    SelectableText(
                      server.accessKey!,
                      style: theme.textTheme.headlineMedium?.copyWith(
                        fontWeight: FontWeight.w700,
                        letterSpacing: 4,
                        fontFeatures: const [FontFeature.tabularFigures()],
                        color: theme.colorScheme.primary,
                      ),
                    ),
                    const SizedBox(height: 16),
                    Row(
                      children: [
                        Expanded(
                          child: FilledButton.icon(
                            onPressed: () => openRemote(server.url!),
                            icon: const Icon(Icons.open_in_new),
                            label: const Text('打开网页'),
                          ),
                        ),
                        const SizedBox(width: 12),
                        Expanded(
                          child: FilledButton.tonalIcon(
                            onPressed: () => copyKey(server.accessKey!),
                            icon: const Icon(Icons.copy_outlined),
                            label: const Text('复制密钥'),
                          ),
                        ),
                      ],
                    ),
                  ],
                ),
              ),
            ),
            const SizedBox(height: 16),
            const Text('在另一台设备打开上述地址并输入密钥。关闭后密钥失效，再次开启会生成新密钥。'),
          ],
          const SizedBox(height: 20),
          const Text('仅在可信局域网中启用。HTTP 会以明文传输密钥及来源内容；Windows 防火墙询问时需允许专用网络访问。'),
        ],
      );
    },
  );
}
