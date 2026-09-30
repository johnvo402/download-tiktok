import unittest

from worker.text_removal.intervals import centers_covering_masked_frames, merge_detected_intervals


class SparseOcrIntervalTests(unittest.TestCase):
    def test_no_hits_returns_no_intervals(self):
        self.assertEqual(merge_detected_intervals([], 2.0, 0.0, 10.0), [])

    def test_adjacent_hits_merge_and_include_padding(self):
        result = merge_detected_intervals([2, 3, 4], 2.0, 0.0, 10.0)
        self.assertEqual(len(result), 1)
        self.assertAlmostEqual(result[0]["startSeconds"], 0.65)
        self.assertAlmostEqual(result[0]["endSeconds"], 3.0)

    def test_one_missed_sample_is_bridged(self):
        result = merge_detected_intervals([2, 4], 2.0, 0.0, 10.0)
        self.assertEqual(len(result), 1)

    def test_one_fps_scan_bridges_one_missed_sample(self):
        result = merge_detected_intervals([1, 3], 1.0, 0.0, 10.0)
        self.assertEqual(len(result), 1)

    def test_long_subtitle_free_gap_splits_intervals(self):
        result = merge_detected_intervals([2, 8], 2.0, 0.0, 10.0)
        self.assertEqual(len(result), 2)
        self.assertLess(result[0]["endSeconds"], result[1]["startSeconds"])

    def test_padding_is_clamped_to_requested_region(self):
        result = merge_detected_intervals([0, 19], 2.0, 5.0, 15.0)
        self.assertEqual(len(result), 2)
        self.assertEqual(result[0]["startSeconds"], 5.0)
        self.assertEqual(result[1]["endSeconds"], 15.0)


class SttnActiveWindowTests(unittest.TestCase):
    def test_only_windows_touching_detected_text_are_processed(self):
        centers = centers_covering_masked_frames(100, [50], radius=5, stride=5)
        self.assertEqual(centers, [45, 50, 55])

    def test_no_mask_skips_all_inpainting_windows(self):
        self.assertEqual(centers_covering_masked_frames(100, [], radius=5, stride=5), [])


if __name__ == "__main__":
    unittest.main()
