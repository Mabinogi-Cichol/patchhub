import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/host/download_rate.dart';

void main() {
  test('download rate uses byte delta and elapsed time, including stalled progress', () {
    final rate = DownloadRate();
    rate.update('connecting', 0, Duration.zero);
    rate.update('downloading', 2000000, const Duration(seconds: 10));
    expect(rate.bytesPerSecond, isNull);
    rate.update('downloading', 5000000, const Duration(milliseconds: 11500));
    expect(rate.bytesPerSecond, 2000000);
    rate.update('downloading', 5000000, const Duration(seconds: 12));
    expect(rate.bytesPerSecond, 0);
  });

  test(
    'a new download and leaving download stage discard the previous rate',
    () {
      final rate = DownloadRate();
      rate.update('downloading', 0, Duration.zero);
      rate.update('downloading', 1000, const Duration(seconds: 1));
      expect(rate.bytesPerSecond, 1000);
      rate.update('downloading', 0, const Duration(seconds: 2));
      expect(rate.bytesPerSecond, isNull);
      rate.update('downloading', 4000, const Duration(seconds: 4));
      expect(rate.bytesPerSecond, 2000);
      rate.update('validating', 4000, const Duration(seconds: 5));
      expect(rate.bytesPerSecond, isNull);
      rate.update('downloading', 5000, const Duration(seconds: 6));
      expect(rate.bytesPerSecond, isNull);
    },
  );
}
