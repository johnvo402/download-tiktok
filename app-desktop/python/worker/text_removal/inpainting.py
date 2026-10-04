"""Buffer-oriented temporal backend. Model settings match the legacy STTN."""
from __future__ import annotations

from abc import ABC, abstractmethod
from collections import OrderedDict
import hashlib
from pathlib import Path
import numpy as np
import cv2

MODEL_WIDTH, MODEL_HEIGHT = 432, 240


def tensor_arrays(images, masks, torch, device):
    resized = [cv2.resize(f, (MODEL_WIDTH, MODEL_HEIGHT), interpolation=cv2.INTER_AREA) for f in images]
    binary = [(cv2.resize(m, (MODEL_WIDTH, MODEL_HEIGHT), interpolation=cv2.INTER_NEAREST) > 0).astype(np.float32) for m in masks]
    image = torch.from_numpy(np.stack(resized).astype(np.float32) / 127.5 - 1.0).permute(0, 3, 1, 2).to(device)
    mask = torch.from_numpy(np.stack(binary)[:, None]).to(device)
    return image, mask


class VideoInpainter(ABC):
    @abstractmethod
    def prepare(self): ...
    @abstractmethod
    def process(self, frames, masks, neighbors, references): ...
    @abstractmethod
    def close(self): ...


class SttnInpainter(VideoInpainter):
    def __init__(self, model_path: Path, device: str = "auto"):
        self.model_path, self.requested = model_path, device
        self.features = OrderedDict()

    def prepare(self):
        import torch
        from .sttn_model import InpaintGenerator
        if self.requested not in {"auto", "cpu", "cuda"}:
            raise ValueError("Thiết bị STTN không hợp lệ.")
        if self.requested == "cuda" and not torch.cuda.is_available():
            raise RuntimeError("STTN NVIDIA chưa sẵn sàng. Chọn CPU hoặc cài thành phần tăng tốc.")
        self.device = torch.device("cuda" if self.requested != "cpu" and torch.cuda.is_available() else "cpu")
        if self.device.type == "cpu":
            torch.set_num_threads(max(1, min(8, torch.get_num_threads())))
        checkpoint = torch.load(str(self.model_path), map_location="cpu", weights_only=True)
        state = checkpoint.get("netG", checkpoint) if isinstance(checkpoint, dict) else checkpoint
        self.model = InpaintGenerator().to(self.device)
        self.model.load_state_dict(state, strict=True)
        self.model.eval()
        self.torch = torch
        self.feature_hits = 0
        self.feature_misses = 0
        if self.device.type == 'cuda':
            torch.cuda.reset_peak_memory_stats()

    def process(self, frames, masks, neighbors, references):
        torch = self.torch
        with torch.inference_mode():
            selected = neighbors + references
            keys = {i: (frames[i].index, frames[i].pts, hashlib.sha256(masks[i].tobytes()).digest()) for i in selected}
            missing = [i for i in selected if keys[i] not in self.features]
            self.feature_misses += len(missing)
            self.feature_hits += len(selected)-len(missing)
            if missing:
                ri, rm = tensor_arrays([frames[i].rgb for i in missing], [masks[i] for i in missing], torch, self.device)
                encoded = self.model.encoder(ri * (1-rm))
                for k, i in enumerate(missing):
                    # Clone each feature so an LRU entry doesn't retain the entire batch storage.
                    self.features[keys[i]] = (encoded[k:k+1].clone(), rm[k:k+1].clone())
            features = torch.cat([self.features[keys[i]][0] for i in selected], dim=0)
            ms = torch.cat([self.features[keys[i]][1] for i in selected], dim=0)
            for i in selected:
                self.features.move_to_end(keys[i])
            # Fixed model-size cache. This never grows with video length or ROI size.
            while len(self.features) > 24:
                self.features.popitem(last=False)
            predicted = self.model.infer(features, ms)[:len(neighbors)]
            output = ((torch.tanh(self.model.decoder(predicted)) + 1) * 127.5).clamp(0, 255)
            return output.permute(0, 2, 3, 1).byte().cpu().numpy()

    def close(self):
        self.features.clear()
        if hasattr(self, "model"):
            del self.model
        if hasattr(self, "torch") and self.device.type == "cuda":
            self.torch.cuda.empty_cache()
