import 'dart:io';

import 'package:flutter/material.dart';

import '../host/steam_artwork.dart';

class GameArtwork extends StatelessWidget {
  const GameArtwork({
    super.key,
    required this.artwork,
    this.kind = ArtworkKind.thumbnail,
  });
  final Future<SteamArtwork> artwork;
  final ArtworkKind kind;

  @override
  Widget build(BuildContext context) => FutureBuilder<SteamArtwork>(
    future: artwork,
    builder: (context, snapshot) {
      final art = snapshot.connectionState == ConnectionState.done
          ? snapshot.data
          : null;
      final path = switch (kind) {
        ArtworkKind.hero => art?.hero,
        ArtworkKind.cover => art?.cover,
        ArtworkKind.thumbnail => art?.thumbnail,
      };
      final scheme = Theme.of(context).colorScheme;
      Widget placeholder([String? diagnostic]) => Tooltip(
        message:
            diagnostic ??
            (art?.diagnostics.isNotEmpty == true
                ? art!.diagnostics.join('\n')
                : '暂无 Steam 艺术作品'),
        child: ColoredBox(
          color: scheme.surfaceContainer,
          child: Center(
            child: Icon(
              Icons.sports_esports_outlined,
              size: kind == ArtworkKind.cover ? 40 : 24,
              color: scheme.onSurfaceVariant.withValues(alpha: 0.5),
            ),
          ),
        ),
      );
      if (path == null) {
        return placeholder(snapshot.hasError ? '${snapshot.error}' : null);
      }
      return LayoutBuilder(
        builder: (context, constraints) {
          final ratio = MediaQuery.devicePixelRatioOf(context);
          final headerThumbnail =
              kind == ArtworkKind.thumbnail && path != art?.cover;
          return Image(
            image: ResizeImage.resizeIfNeeded(
              headerThumbnail ? null : (constraints.maxWidth * ratio).ceil(),
              headerThumbnail ? (constraints.maxHeight * ratio).ceil() : null,
              _ArtworkFileImage(File(path), art!.revision),
            ),
            key: ValueKey(path),
            fit: BoxFit.cover,
            alignment: kind == ArtworkKind.hero
                ? Alignment.centerRight
                : Alignment.center,
            excludeFromSemantics: true,
            errorBuilder: (context, error, stack) =>
                placeholder('图片无法显示：$path\n$error'),
          );
        },
      );
    },
  );
}

enum ArtworkKind { hero, cover, thumbnail }

// An explicit library refresh must also display edits made to the same image file.
class _ArtworkFileImage extends FileImage {
  const _ArtworkFileImage(super.file, this.revision);
  final int revision;
  @override
  bool operator ==(Object other) =>
      other is _ArtworkFileImage &&
      super == other &&
      revision == other.revision;
  @override
  int get hashCode => Object.hash(super.hashCode, revision);
}
