import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:steam_patch_windows/host/steam_artwork.dart';
import 'package:steam_patch_windows/presentation/game_artwork.dart';

void main() {
  testWidgets(
    'switching games clears previous artwork while the new result is pending',
    (tester) async {
      Widget image(Future<SteamArtwork> art) => MaterialApp(
        home: SizedBox(
          width: 100,
          height: 100,
          child: GameArtwork(artwork: art),
        ),
      );
      final first = Future.value(
        const SteamArtwork(diagnostics: ['游戏 A 图片不可用']),
      );
      await tester.pumpWidget(image(first));
      await tester.pumpAndSettle();
      expect(find.byTooltip('游戏 A 图片不可用'), findsOneWidget);
      final next = Completer<SteamArtwork>();
      await tester.pumpWidget(image(next.future));
      await tester.pump();
      expect(find.byTooltip('游戏 A 图片不可用'), findsNothing);
      expect(find.byTooltip('暂无 Steam 艺术作品'), findsOneWidget);
      next.complete(const SteamArtwork(diagnostics: ['游戏 B 图片不可用']));
      await tester.pumpAndSettle();
      expect(find.byTooltip('游戏 B 图片不可用'), findsOneWidget);
      expect(tester.takeException(), isNull);
    },
  );
}
