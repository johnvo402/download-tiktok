"""Reusable interval masks. 255 = remove, 0 = preserve, RGB is explicit."""
from __future__ import annotations

from dataclasses import dataclass
from bisect import bisect_right
import difflib
import cv2
import numpy as np


@dataclass(frozen=True)
class MaskProfile:
    expansion: float = 6.0  # Pixels at reference height 1080, not fixed crop pixels.
    feather: float = 1.5


def same_subtitle(a: dict, b: dict) -> bool:
    aa, bb = np.asarray(a["boxes"]), np.asarray(b["boxes"])
    if aa.shape != bb.shape or not aa.size:
        return False
    scale = max(8, float(np.ptp(aa[..., 1])))
    geometry = float(np.mean(np.abs(aa - bb))) / scale < 0.2
    ta, tb = a.get("text", ""), b.get("text", "")
    return geometry and (not ta or not tb or difflib.SequenceMatcher(None, ta, tb).ratio() >= 0.75)


def stabilize(a: list, b: list) -> list:
    aa, bb = np.asarray(a, np.float32), np.asarray(b, np.float32)
    return (aa * 0.75 + bb * 0.25).tolist() if aa.shape == bb.shape else b


def make_mask(width: int, height: int, boxes: list, source_height: int,
              profile: MaskProfile = MaskProfile(), *, full_region: dict | None = None) -> tuple[np.ndarray, np.ndarray]:
    mask = np.zeros((height, width), np.uint8)
    if full_region is not None:
        x, y, w, h = (int(full_region[k]) for k in ("x", "y", "width", "height"))
        mask[max(0, y):min(height, y+h), max(0, x):min(width, x+w)] = 255
    else:
        for box in boxes:
            polygon = np.rint(np.asarray(box, np.float32)).astype(np.int32)
            cv2.fillPoly(mask, [polygon], 255)
        radius = max(1, round(profile.expansion * source_height / 1080))
        mask = cv2.dilate(mask, cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (2*radius+1, 2*radius+1)))
    sigma = profile.feather * source_height / 1080
    alpha = cv2.GaussianBlur(mask, (0, 0), sigmaX=sigma) if sigma > 0 else mask.copy()
    # Feather inside the removal mask. Never reveal original glyph pixels outside it.
    alpha[mask == 0] = 0
    return mask, alpha


def scene_ids(frames: list) -> list[int]:
    ids, previous, scene = [], None, 0
    for f in frames:
        small = cv2.resize(f.rgb, (64, 36), interpolation=cv2.INTER_AREA).astype(np.float32)
        if previous is not None and np.mean(np.abs(small - previous)) > 48:
            scene += 1
        ids.append(scene)
        previous = small
    return ids


def context_masks(frames, intervals, starts, current_index, binary, width, height,
                  local, source_height, profile, mode):
    """Adjacent captions must not be treated as clean temporal reference pixels."""
    empty = np.zeros_like(binary)
    by_interval = {current_index: binary}
    masks = []
    for frame in frames:
        i = bisect_right(starts, frame.pts) - 1
        if i < 0 or frame.pts >= intervals[i]['endSeconds']:
            masks.append(empty)
            continue
        if mode == 'fullRegion':
            masks.append(binary)
            continue
        if i not in by_interval:
            boxes = [(np.asarray(b, np.float32) + [local['x'], local['y']]).tolist() for b in intervals[i]['boxes']]
            by_interval[i] = make_mask(width, height, boxes, source_height, profile)[0]
        masks.append(by_interval[i])
    return masks


def plan_windows(total: int, active: list[int], scenes: list[int], masks: list[np.ndarray],
                 *, radius: int = 5, stride: int = 5, references: int = 8, indices: list[int] | None = None):
    active_set = set(active)
    centers = [i for i in range(total) if (indices[i] if indices else i) % stride == 0
               if any(j in active_set for j in range(max(0, i-radius), min(total, i+radius+1)))]
    # A very short scene between two cuts may contain no stride-aligned center.
    for i in active:
        if not any(scenes[c] == scenes[i] and abs(c-i) <= radius for c in centers):
            centers.append(i)
    for center in sorted(set(centers)):
        neighbors = [i for i in range(max(0, center-radius), min(total, center+radius+1)) if scenes[i] == scenes[center]]
        if not any(i in active_set for i in neighbors):
            continue
        candidates = [i for i in range(total) if scenes[i] == scenes[center] and i not in neighbors]
        # Unmasked frames are preferred, with deterministic temporal spread.
        clean = [i for i in candidates if not np.any(masks[i])]
        dirty = [i for i in candidates if np.any(masks[i])]
        refs = []
        for choices in (clean, dirty):
            count = min(references-len(refs), len(choices))
            if count:
                refs.extend(choices[i] for i in np.linspace(0, len(choices)-1, count, dtype=int))
        yield neighbors, refs
