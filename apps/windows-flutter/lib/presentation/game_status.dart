import 'package:flutter/material.dart';

import '../models/library.dart';
import 'library_presentation.dart';
import 'patch_status_tone.dart';

class GameStatus extends StatelessWidget {
  const GameStatus({
    super.key,
    required this.row,
    this.emphasized = false,
    this.querying = false,
  });
  final GameRow row;
  final bool emphasized;
  final bool querying;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final presentation = rowStatus(row, querying: querying);
    final color = presentation.tone.color(scheme);
    final installed = presentation.tone == PatchStatusTone.success;
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
            presentation.label,
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
