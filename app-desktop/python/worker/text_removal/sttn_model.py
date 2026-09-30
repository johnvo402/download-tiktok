"""Minimal STTN generator for subtitle inpainting.

The generator architecture follows researchmm/STTN (MIT, Yanhong Zeng et al.);
the discriminator and training-only helpers are intentionally omitted.
"""

from __future__ import annotations

import math

import torch
from torch import nn
from torch.nn import functional as F


class Attention(nn.Module):
    def forward(self, query, key, value, mask):
        scores = torch.matmul(query, key.transpose(-2, -1)) / math.sqrt(query.size(-1))
        scores = scores.masked_fill(mask, -1e9)
        probabilities = F.softmax(scores, dim=-1)
        return torch.matmul(probabilities, value)


class MultiHeadedAttention(nn.Module):
    def __init__(self, patch_sizes, channels):
        super().__init__()
        self.patch_sizes = patch_sizes
        self.query_embedding = nn.Conv2d(channels, channels, 1)
        self.key_embedding = nn.Conv2d(channels, channels, 1)
        self.value_embedding = nn.Conv2d(channels, channels, 1)
        self.output_linear = nn.Sequential(
            nn.Conv2d(channels, channels, 3, padding=1), nn.LeakyReLU(0.2, inplace=True)
        )
        self.attention = Attention()

    def forward(self, features, masks, batch, channels):
        batch_time, _, height, width = features.size()
        time = batch_time // batch
        head_channels = channels // len(self.patch_sizes)
        query = self.query_embedding(features)
        key = self.key_embedding(features)
        value = self.value_embedding(features)
        output = []
        query_heads = torch.chunk(query, len(self.patch_sizes), dim=1)
        key_heads = torch.chunk(key, len(self.patch_sizes), dim=1)
        value_heads = torch.chunk(value, len(self.patch_sizes), dim=1)

        for (patch_width, patch_height), q, k, v in zip(
            self.patch_sizes, query_heads, key_heads, value_heads
        ):
            out_width = width // patch_width
            out_height = height // patch_height
            mask = masks.view(batch, time, 1, out_height, patch_height, out_width, patch_width)
            mask = mask.permute(0, 1, 3, 5, 2, 4, 6).contiguous()
            mask = mask.view(batch, time * out_height * out_width, patch_height * patch_width)
            mask = (mask.mean(-1) > 0.5).unsqueeze(1).expand(-1, mask.size(1), -1)

            def patchify(tensor):
                tensor = tensor.view(batch, time, head_channels, out_height, patch_height, out_width, patch_width)
                return tensor.permute(0, 1, 3, 5, 2, 4, 6).contiguous().view(
                    batch, time * out_height * out_width, head_channels * patch_height * patch_width
                )

            attended = self.attention(patchify(q), patchify(k), patchify(v), mask)
            attended = attended.view(batch, time, out_height, out_width, head_channels, patch_height, patch_width)
            attended = attended.permute(0, 1, 4, 2, 5, 3, 6).contiguous().view(
                batch_time, head_channels, height, width
            )
            output.append(attended)
        return features + self.output_linear(torch.cat(output, dim=1))


class TransformerBlock(nn.Module):
    def __init__(self, patch_sizes, channels):
        super().__init__()
        self.attention = MultiHeadedAttention(patch_sizes, channels)
        self.feed_forward = FeedForward(channels)

    def forward(self, state):
        features, masks, batch, channels = state["x"], state["m"], state["b"], state["c"]
        features = self.attention(features, masks, batch, channels)
        features = features + self.feed_forward(features)
        return {"x": features, "m": masks, "b": batch, "c": channels}


class FeedForward(nn.Module):
    def __init__(self, channels):
        super().__init__()
        # Preserve the official checkpoint's transformer.N.feed_forward.conv.* keys.
        self.conv = nn.Sequential(
            nn.Conv2d(channels, channels, 3, padding=2, dilation=2),
            nn.LeakyReLU(0.2, inplace=True),
            nn.Conv2d(channels, channels, 3, padding=1),
            nn.LeakyReLU(0.2, inplace=True),
        )

    def forward(self, state):
        return self.conv(state)


class UpsampleConv(nn.Module):
    def __init__(self, in_channels, out_channels):
        super().__init__()
        self.conv = nn.Conv2d(in_channels, out_channels, 3, padding=1)

    def forward(self, value):
        return self.conv(F.interpolate(value, scale_factor=2, mode="bilinear", align_corners=True))


class InpaintGenerator(nn.Module):
    """STTN generator with the checkpoint-compatible `netG` state names."""

    def __init__(self):
        super().__init__()
        channels = 256
        patch_sizes = [(108, 60), (36, 20), (18, 10), (9, 5)]
        self.transformer = nn.Sequential(*(TransformerBlock(patch_sizes, channels) for _ in range(8)))
        self.encoder = nn.Sequential(
            nn.Conv2d(3, 64, 3, stride=2, padding=1), nn.LeakyReLU(0.2, inplace=True),
            nn.Conv2d(64, 64, 3, padding=1), nn.LeakyReLU(0.2, inplace=True),
            nn.Conv2d(64, 128, 3, stride=2, padding=1), nn.LeakyReLU(0.2, inplace=True),
            nn.Conv2d(128, channels, 3, padding=1), nn.LeakyReLU(0.2, inplace=True),
        )
        self.decoder = nn.Sequential(
            UpsampleConv(channels, 128), nn.LeakyReLU(0.2, inplace=True),
            nn.Conv2d(128, 64, 3, padding=1), nn.LeakyReLU(0.2, inplace=True),
            UpsampleConv(64, 64), nn.LeakyReLU(0.2, inplace=True),
            nn.Conv2d(64, 3, 3, padding=1),
        )

    def infer(self, features, masks):
        _, channels, _, _ = features.size()
        masks = F.interpolate(masks, scale_factor=0.25)
        return self.transformer({"x": features, "m": masks, "b": 1, "c": channels})["x"]
