from __future__ import annotations

import math


def merge_detected_intervals(
    detected_indices: list[int],
    fps: float,
    start_seconds: float,
    end_seconds: float,
    *,
    pad_before: float = 0.35,
    pad_after: float = 0.5,
    merge_gap: float = 0.6,
) -> list[dict[str, float]]:
    """Convert sparse OCR hits into padded, merged video-time spans.

    A hit represents one sampled frame and therefore covers one sampling period.
    A single missed sample is bridged, while longer subtitle-free gaps stay out
    of the expensive full-rate inpainting pass.
    """
    if not math.isfinite(fps) or fps <= 0 or not math.isfinite(start_seconds) or not math.isfinite(end_seconds):
        raise ValueError("OCR scan timing is invalid.")
    if end_seconds <= start_seconds:
        return []
    period = 1.0 / fps
    effective_merge_gap = max(merge_gap, period * 1.1)
    indices = sorted({int(index) for index in detected_indices if int(index) >= 0})
    if not indices:
        return []

    groups: list[list[int]] = [[indices[0]]]
    for index in indices[1:]:
        previous_end = start_seconds + (groups[-1][-1] + 1) * period
        current_start = start_seconds + index * period
        if current_start - previous_end <= effective_merge_gap:
            groups[-1].append(index)
        else:
            groups.append([index])

    spans = []
    for group in groups:
        span_start = max(start_seconds, start_seconds + group[0] * period - pad_before)
        span_end = min(end_seconds, start_seconds + (group[-1] + 1) * period + pad_after)
        if span_end > span_start:
            spans.append({"startSeconds": span_start, "endSeconds": span_end})
    return spans


def centers_covering_masked_frames(
    total: int,
    active_indices: list[int],
    *,
    radius: int,
    stride: int,
) -> list[int]:
    """Choose only temporal model windows that touch frames with an OCR mask."""
    if total <= 0 or not active_indices or radius < 0 or stride <= 0:
        return []
    active = set(active_indices)
    return [center for center in range(0, total, stride)
            if any(index in active for index in range(max(0, center - radius), min(total, center + radius + 1)))]
