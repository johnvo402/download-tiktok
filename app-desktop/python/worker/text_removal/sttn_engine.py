from __future__ import annotations

from pathlib import Path
from typing import Any, Callable

import cv2
import numpy as np
from PIL import Image

from worker.text_removal.intervals import centers_covering_masked_frames

MODEL_WIDTH = 432
MODEL_HEIGHT = 240
WINDOW_RADIUS = 5
WINDOW_STRIDE = 5
MAX_REFERENCE_FRAMES = 8


def _load_frame(path: Path) -> np.ndarray:
    # np.asarray(PIL.Image) may expose a read-only view. STTN blends predicted
    # pixels into this frame in _blend_prediction, so return an owned writable
    # buffer instead.
    with Image.open(path) as image:
        return np.array(image.convert("RGB"), dtype=np.uint8, copy=True)


def _load_mask(path: Path) -> np.ndarray:
    with Image.open(path) as image:
        return np.array(image.convert("L"), dtype=np.uint8, copy=True)


def _tensor_inputs(frame_paths: list[Path], mask_paths: list[Path], indices: list[int], torch, device):
    images: list[np.ndarray] = []
    masks: list[np.ndarray] = []
    for index in indices:
        frame = _load_frame(frame_paths[index])
        mask = _load_mask(mask_paths[index])
        frame = cv2.resize(frame, (MODEL_WIDTH, MODEL_HEIGHT), interpolation=cv2.INTER_AREA)
        mask = cv2.resize(mask, (MODEL_WIDTH, MODEL_HEIGHT), interpolation=cv2.INTER_NEAREST)
        images.append(frame)
        masks.append((mask > 0).astype(np.float32))
    image_tensor = torch.from_numpy(np.stack(images).astype(np.float32) / 127.5 - 1.0)
    image_tensor = image_tensor.permute(0, 3, 1, 2).to(device)
    mask_tensor = torch.from_numpy(np.stack(masks)[:, None]).to(device)
    return image_tensor, mask_tensor


def _reference_indices(total: int, excluded: set[int]) -> list[int]:
    available = [index for index in np.linspace(0, total - 1, min(total, MAX_REFERENCE_FRAMES + 2), dtype=int)
                 if index not in excluded]
    if len(available) > MAX_REFERENCE_FRAMES:
        positions = np.linspace(0, len(available) - 1, MAX_REFERENCE_FRAMES, dtype=int)
        available = [available[position] for position in positions]
    return list(dict.fromkeys(available))


def _blend_prediction(output_path: Path, current: np.ndarray, predicted: np.ndarray,
                      mask: np.ndarray, counts: dict[int, int], index: int) -> None:
    count = counts.get(index, 0)
    if count and output_path.is_file():
        previous = np.asarray(Image.open(output_path).convert("RGBA"))[..., :3]
        alpha = mask > 0
        current[alpha] = np.rint((previous[alpha].astype(np.float32) * count + predicted[alpha].astype(np.float32)) / (count + 1)).astype(np.uint8)
    else:
        alpha = mask > 0
        current[alpha] = predicted[alpha]
    counts[index] = count + 1
    alpha = cv2.GaussianBlur(mask, (0, 0), sigmaX=0.75)
    Image.fromarray(np.dstack((current, alpha)), mode="RGBA").save(output_path, format="PNG", compress_level=1)


def process_sttn_frames(
    frames_dir: Path,
    masks_dir: Path,
    output_dir: Path,
    model_path: Path,
    emit: Callable[[dict[str, object]], None],
    runtime_cache: dict[str, Any] | None = None,
) -> dict[str, Any]:
    import torch
    runtime_cache = runtime_cache if runtime_cache is not None else {}

    frame_paths = sorted(frames_dir.glob("frame_*.jpg"))
    mask_paths = [masks_dir / f"mask_{index + 1:08d}.png" for index in range(len(frame_paths))]
    total = len(frame_paths)
    if not total:
        raise RuntimeError("Không có khung hình video để xử lý bằng STTN.")
    if not model_path.is_file():
        raise RuntimeError("Chưa tải mô hình STTN. Hãy mở Cài đặt ứng dụng → Nhận dạng lời thoại để cài model.")
    active_indices = [index for index, path in enumerate(mask_paths) if np.any(_load_mask(path))]
    if not active_indices:
        return {"frames": total, "maskedFrames": 0, "device": "cuda" if torch.cuda.is_available() else "cpu"}
    cached_runtime = runtime_cache.get("sttn_runtime")
    if cached_runtime is not None:
        model, device = cached_runtime
    else:
        if not torch.cuda.is_available():
            device = torch.device("cpu")
            torch.set_num_threads(max(1, min(8, torch.get_num_threads())))
        else:
            device = torch.device("cuda")
        try:
            from worker.text_removal.sttn_model import InpaintGenerator
            checkpoint = torch.load(str(model_path), map_location="cpu", weights_only=True)
            state = checkpoint.get("netG", checkpoint) if isinstance(checkpoint, dict) else checkpoint
            model = InpaintGenerator().to(device)
            model.load_state_dict(state, strict=True)
            model.eval()
        except Exception as error:
            raise RuntimeError(f"Không thể nạp mô hình STTN: {error}") from error
        runtime_cache["sttn_runtime"] = (model, device)

    output_dir.mkdir(parents=True, exist_ok=True)
    output_paths = [output_dir / f"clean_{index + 1:08d}.png" for index in range(total)]
    counts: dict[int, int] = {}
    references = _reference_indices(total, set())
    # Cache only a few global reference features. This preserves longer-range
    # context without keeping every frame's feature tensor in RAM/VRAM.
    with torch.inference_mode():
        if references:
            ref_images, ref_masks = _tensor_inputs(frame_paths, mask_paths, references, torch, device)
            ref_features = model.encoder(ref_images * (1 - ref_masks))
        else:
            ref_masks = None
            ref_features = None
        centers = centers_covering_masked_frames(total, active_indices, radius=WINDOW_RADIUS, stride=WINDOW_STRIDE)
        for pass_index, center in enumerate(centers):
            neighbors = list(range(max(0, center - WINDOW_RADIUS), min(total, center + WINDOW_RADIUS + 1)))
            refs = [index for index in references if index not in set(neighbors)]
            neighbor_images, neighbor_masks = _tensor_inputs(frame_paths, mask_paths, neighbors, torch, device)
            neighbor_features = model.encoder(neighbor_images * (1 - neighbor_masks))
            all_masks = neighbor_masks
            all_features = neighbor_features
            if refs and ref_features is not None and ref_masks is not None:
                ref_positions = [references.index(index) for index in refs]
                ref_position_tensor = torch.tensor(ref_positions, dtype=torch.long, device=device)
                all_features = torch.cat((neighbor_features, ref_features.index_select(0, ref_position_tensor)), dim=0)
                all_masks = torch.cat((neighbor_masks, ref_masks.index_select(0, ref_position_tensor)), dim=0)
            prediction_features = model.infer(all_features, all_masks)[:len(neighbors)]
            predictions = (torch.tanh(model.decoder(prediction_features)) + 1) * 127.5
            predictions = predictions.clamp(0, 255).permute(0, 2, 3, 1).byte().cpu().numpy()

            for local_index, frame_index in enumerate(neighbors):
                frame = _load_frame(frame_paths[frame_index])
                mask = _load_mask(mask_paths[frame_index])
                if not np.any(mask):
                    if not output_paths[frame_index].exists():
                        Image.fromarray(np.dstack((frame, np.zeros(mask.shape, dtype=np.uint8))), mode="RGBA").save(
                            output_paths[frame_index], format="PNG", compress_level=1
                        )
                    continue
                prediction = cv2.resize(
                    predictions[local_index], (frame.shape[1], frame.shape[0]), interpolation=cv2.INTER_CUBIC
                )
                _blend_prediction(output_paths[frame_index], frame, prediction, mask, counts, frame_index)

            progress = (pass_index + 1) / max(1, len(centers))
            if pass_index % max(1, len(centers) // 100) == 0 or pass_index + 1 == len(centers):
                emit({"type": "progress", "job": "text_removal", "stage": "sttn",
                      "progress": progress, "message": f"Đang phục hồi theo thời gian bằng STTN · {min(total, center + WINDOW_RADIUS + 1)}/{total} khung hình"})
        # FFmpeg reads a contiguous numbered image sequence. Frames outside every
        # active STTN window are represented by transparent overlays and pass through.
        for index, output_path in enumerate(output_paths):
            if output_path.exists():
                continue
            frame = _load_frame(frame_paths[index])
            mask = _load_mask(mask_paths[index])
            Image.fromarray(np.dstack((frame, np.zeros(mask.shape, dtype=np.uint8))), mode="RGBA").save(
                output_path, format="PNG", compress_level=1
            )
        if device.type == "cuda":
            torch.cuda.empty_cache()
    if not counts:
        raise RuntimeError("OCR không phát hiện chữ trong vùng đã chọn. Hãy chỉnh vùng xử lý rồi thử lại.")
    return {"frames": total, "maskedFrames": len(counts), "device": str(device)}
