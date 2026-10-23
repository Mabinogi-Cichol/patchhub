import 'package:flutter/material.dart';

import '../models/library.dart';
import 'library_presentation.dart';

class GameStatus extends StatelessWidget {
  const GameStatus({super.key, required this.row, this.emphasized = false});
  final GameRow row;
  final bool emphasized;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final installed = row.patches.any(
      (p) => installedStatus(p['detection']['status']),
    );
    final color = installed
        ? (Theme.of(context).brightness == Brightness.dark
              ? const Color(0xff70d7a7)
              : const Color(0xff17633e))
        : row.matched
        ? scheme.secondary
        : scheme.onSurfaceVariant;
    final status = Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        Icon(
          installed ? Icons.check_circle_outline : Icons.circle,
          size: installed ? 13 : 7,
          color: color,
        ),
        const SizedBox(width: 6),
        Flexible(
          child: Text(
            rowSummary(row),
            style: TextStyle(color: color, fontSize: emphasized ? 13 : 12),
          ),
        ),
      ],
    );
    return emphasized
        ? Container(
            padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 5),
            decoration: BoxDecoration(
              color: color.withValues(alpha: 0.10),
              borderRadius: BorderRadius.circular(20),
            ),
            child: status,
          )
        : status;
  }
}
