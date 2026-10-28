"""Apollo: band-sequence modelling for music restoration of compressed audio.

The network definition below is adapted from the Look2Hear Apollo
repository (https://github.com/JusperLee/Apollo, `look2hear/models/apollo.py`)
by Kai Li and Yi Luo (Tsinghua University, Tencent AI Lab), licensed under
the Creative Commons Attribution-ShareAlike 4.0 International License. The
changes: the Hugging Face mixin and debug print are removed, and the
modules are built inside `build_apollo` so PyTorch is imported only when
the model is used. The weights (JusperLee/Apollo on Hugging Face, same
licence) are not part of this repository; see `weights.py`.

Apollo was trained on MP3 encodes from 24 to 128 kbps at 44.1 kHz. It
generates plausible missing detail; it does not recover the original.
"""

from __future__ import annotations

import time
from pathlib import Path

import numpy as np

from ..weights import APOLLO, ensure_weights

SAMPLE_RATE = 44_100
VERSION = "1.0"


def build_apollo(sr: int = SAMPLE_RATE, win: int = 20, feature_dim: int = 256, layer: int = 6):
    import torch
    import torch.nn as nn
    import torch.nn.functional as F

    class RMSNorm(nn.Module):
        def __init__(self, dimension, groups=1):
            super().__init__()
            self.weight = nn.Parameter(torch.ones(dimension))
            self.groups = groups
            self.eps = 1e-5

        def forward(self, input):
            B, N, T = input.shape
            input_float = input.reshape(B, self.groups, -1, T).float()
            input_norm = input_float * torch.rsqrt(
                input_float.pow(2).mean(-2, keepdim=True) + self.eps
            )
            return input_norm.type_as(input).reshape(B, N, T) * self.weight.reshape(1, -1, 1)

    class Roformer(nn.Module):
        def __init__(
            self,
            input_size,
            hidden_size,
            num_head=8,
            theta=10000,
            window=10000,
            input_drop=0.0,
            attention_drop=0.0,
            causal=True,
        ):
            super().__init__()
            self.input_size = input_size
            self.hidden_size = hidden_size // num_head
            self.num_head = num_head
            self.theta = theta
            self.window = window
            cos_freq, sin_freq = self._calc_rotary_emb()
            self.register_buffer("cos_freq", cos_freq)
            self.register_buffer("sin_freq", sin_freq)
            self.attention_drop = attention_drop
            self.causal = causal
            self.eps = 1e-5
            self.input_norm = RMSNorm(self.input_size)
            self.input_drop = nn.Dropout(p=input_drop)
            self.weight = nn.Conv1d(
                self.input_size, self.hidden_size * self.num_head * 3, 1, bias=False
            )
            self.output = nn.Conv1d(
                self.hidden_size * self.num_head, self.input_size, 1, bias=False
            )
            self.MLP = nn.Sequential(
                RMSNorm(self.input_size),
                nn.Conv1d(self.input_size, self.input_size * 8, 1, bias=False),
                nn.SiLU(),
            )
            self.MLP_output = nn.Conv1d(self.input_size * 4, self.input_size, 1, bias=False)

        def _calc_rotary_emb(self):
            freq = 1.0 / (
                self.theta
                ** (
                    torch.arange(0, self.hidden_size, 2)[: (self.hidden_size // 2)]
                    / self.hidden_size
                )
            )
            freq = freq.reshape(1, -1)
            pos = torch.arange(0, self.window).reshape(-1, 1)
            cos_freq = torch.cos(pos * freq)
            sin_freq = torch.sin(pos * freq)
            cos_freq = torch.stack([cos_freq] * 2, -1).reshape(self.window, self.hidden_size)
            sin_freq = torch.stack([sin_freq] * 2, -1).reshape(self.window, self.hidden_size)
            return cos_freq, sin_freq

        def _add_rotary_sequence(self, feature):
            T, N = feature.shape[-2:]
            feature_reshape = feature.reshape(-1, T, N)
            cos_freq = self.cos_freq[:T]
            sin_freq = self.sin_freq[:T]
            reverse_sign = torch.tensor([-1.0, 1.0], device=feature.device, dtype=feature.dtype)
            feature_reshape_neg = (
                torch.flip(feature_reshape.reshape(-1, N // 2, 2), [-1])
                * reverse_sign.reshape(1, 1, 2)
            ).reshape(-1, T, N)
            feature_rope = feature_reshape * cos_freq.unsqueeze(
                0
            ) + feature_reshape_neg * sin_freq.unsqueeze(0)
            return feature_rope.reshape(feature.shape)

        def forward(self, input):
            B, _, T = input.shape
            weight = (
                self.weight(self.input_drop(self.input_norm(input)))
                .reshape(B, self.num_head, self.hidden_size * 3, T)
                .mT
            )
            Q, K, V = torch.split(weight, self.hidden_size, dim=-1)
            Q_rot = self._add_rotary_sequence(Q)
            K_rot = self._add_rotary_sequence(K)
            attention_output = F.scaled_dot_product_attention(
                Q_rot.contiguous(),
                K_rot.contiguous(),
                V.contiguous(),
                dropout_p=self.attention_drop,
                is_causal=self.causal,
            )
            attention_output = attention_output.mT.reshape(B, -1, T)
            output = self.output(attention_output) + input
            gate, z = self.MLP(output).chunk(2, dim=1)
            output = output + self.MLP_output(F.silu(gate) * z)
            return output, (K_rot, V)

    class ConvActNorm1d(nn.Module):
        def __init__(self, in_channel, hidden_channel, kernel=7, causal=False):
            super().__init__()
            self.kernel = kernel
            self.causal = causal
            padding = kernel - 1 if causal else (kernel - 1) // 2
            self.conv = nn.Sequential(
                nn.Conv1d(in_channel, in_channel, kernel, padding=padding, groups=in_channel),
                RMSNorm(in_channel),
                nn.Conv1d(in_channel, hidden_channel, 1),
                nn.SiLU(),
                nn.Conv1d(hidden_channel, in_channel, 1),
            )

        def forward(self, input):
            output = self.conv(input)
            if self.causal:
                output = output[..., : -self.kernel + 1]
            return input + output

    class ICB(nn.Module):
        def __init__(self, in_channel, kernel=7, causal=False):
            super().__init__()
            self.blocks = nn.Sequential(
                ConvActNorm1d(in_channel, in_channel * 4, kernel, causal=causal),
                ConvActNorm1d(in_channel, in_channel * 4, kernel, causal=causal),
                ConvActNorm1d(in_channel, in_channel * 4, kernel, causal=causal),
            )

        def forward(self, input):
            return self.blocks(input)

    class BSNet(nn.Module):
        def __init__(self, feature_dim, kernel=7):
            super().__init__()
            self.feature_dim = feature_dim
            self.band_net = Roformer(
                self.feature_dim, self.feature_dim, num_head=8, window=100, causal=False
            )
            self.seq_net = ICB(self.feature_dim, kernel=kernel)

        def forward(self, input):
            B, nband, N, T = input.shape
            band_input = input.permute(0, 3, 2, 1).reshape(B * T, -1, nband)
            band_output, _ = self.band_net(band_input)
            band_output = band_output.reshape(B, T, -1, nband).permute(0, 3, 2, 1)
            output = self.seq_net(band_output.reshape(B * nband, -1, T)).reshape(B, nband, -1, T)
            return output

    class Apollo(nn.Module):
        def __init__(self, sr, win, feature_dim, layer):
            super().__init__()
            self.sr = sr
            self.win = int(sr * win // 1000)
            self.stride = self.win // 2
            self.enc_dim = self.win // 2 + 1
            self.feature_dim = feature_dim
            self.eps = torch.finfo(torch.float32).eps
            bandwidth = int(self.win / 160)
            self.band_width = [bandwidth] * 79
            self.band_width.append(self.enc_dim - int(np.sum(self.band_width)))
            self.nband = len(self.band_width)
            self.BN = nn.ModuleList(
                [
                    nn.Sequential(
                        RMSNorm(self.band_width[i] * 2 + 1),
                        nn.Conv1d(self.band_width[i] * 2 + 1, self.feature_dim, 1),
                    )
                    for i in range(self.nband)
                ]
            )
            self.net = nn.Sequential(*[BSNet(self.feature_dim) for _ in range(layer)])
            self.output = nn.ModuleList(
                [
                    nn.Sequential(
                        RMSNorm(self.feature_dim),
                        nn.Conv1d(self.feature_dim, self.band_width[i] * 4, 1),
                        nn.GLU(dim=1),
                    )
                    for i in range(self.nband)
                ]
            )

        def spec_band_split(self, input):
            B, nch, nsample = input.shape
            spec = torch.stft(
                input.view(B * nch, nsample),
                n_fft=self.win,
                hop_length=self.stride,
                window=torch.hann_window(self.win).to(input.device),
                return_complex=True,
            )
            subband_spec_norm, subband_power = [], []
            band_idx = 0
            for i in range(self.nband):
                this_spec = spec[:, band_idx : band_idx + self.band_width[i]]
                power = (this_spec.abs().pow(2).sum(1) + self.eps).sqrt().unsqueeze(1)
                subband_power.append(power)
                subband_spec_norm.append(
                    torch.complex(this_spec.real / power, this_spec.imag / power)
                )
                band_idx += self.band_width[i]
            return subband_spec_norm, torch.cat(subband_power, 1)

        def forward(self, input):
            B, nch, nsample = input.shape
            subband_spec_norm, subband_power = self.spec_band_split(input)
            features = []
            for i in range(self.nband):
                concat_spec = torch.cat(
                    [
                        subband_spec_norm[i].real,
                        subband_spec_norm[i].imag,
                        torch.log(subband_power[:, i].unsqueeze(1)),
                    ],
                    1,
                )
                features.append(self.BN[i](concat_spec))
            feature = self.net(torch.stack(features, 1))
            est_spec = []
            for i in range(self.nband):
                this_RI = self.output[i](feature[:, i]).view(B * nch, 2, self.band_width[i], -1)
                est_spec.append(torch.complex(this_RI[:, 0], this_RI[:, 1]))
            est_spec = torch.cat(est_spec, 1)
            return torch.istft(
                est_spec,
                n_fft=self.win,
                hop_length=self.stride,
                window=torch.hann_window(self.win).to(input.device),
                length=nsample,
            ).view(B, nch, -1)

    return Apollo(sr, win, feature_dim, layer)


def load_state(path: Path):
    """Reads the checkpoint the pinned SHA-256 already verified."""
    import torch

    try:
        conf = torch.load(path, map_location="cpu", weights_only=True)
    except Exception:  # noqa: BLE001 - older checkpoints need the full unpickler
        # Only reached for the exact file the pinned hash names.
        conf = torch.load(path, map_location="cpu", weights_only=False)
    state = conf.get("state_dict", conf) if isinstance(conf, dict) else conf
    return state


def pick_device(requested: str) -> tuple[str, str]:
    import torch

    if requested in ("auto", "cuda") and torch.cuda.is_available():
        return "cuda", torch.cuda.get_device_name(0)
    if requested == "cuda":
        print("CUDA was requested but is not available; using the CPU (slow).")
    return "cpu", "CPU"


class ApolloModel:
    name = "apollo"
    version = VERSION
    sample_rate = SAMPLE_RATE
    license = APOLLO.license
    # 10 s chunks, 1 s crossfades and 1 s of context each side keep the
    # seams clean and the GPU memory bounded.
    chunk = SAMPLE_RATE * 10
    overlap = SAMPLE_RATE
    pad = SAMPLE_RATE

    def __init__(self, network, device: str, device_name: str):
        self.network = network
        self.device = device
        self.device_name = device_name

    @classmethod
    def load(cls, device: str, cache_dir: Path, offline: bool) -> ApolloModel:
        import torch

        weights = ensure_weights(APOLLO, Path(cache_dir), offline=offline)
        device, device_name = pick_device(device)
        if device == "cpu":
            print("No CUDA GPU: Apollo runs on the CPU. Enhancing works, but slowly.")
        network = build_apollo()
        network.load_state_dict(load_state(weights))
        network = network.to(device).eval()
        torch.set_grad_enabled(False)
        return cls(network, device, device_name)

    def enhance(self, audio: np.ndarray) -> np.ndarray:
        import torch

        with torch.inference_mode():
            tensor = torch.from_numpy(np.ascontiguousarray(audio, dtype=np.float32))
            output = self.network(tensor.unsqueeze(0).to(self.device))
            return output.squeeze(0).float().cpu().numpy().astype(np.float64)


def measure_realtime_factor(model, seconds: float = 4.0) -> float:
    """Seconds of processing per second of audio, after a warm-up run."""
    rng = np.random.default_rng(7)
    sample = rng.standard_normal((2, int(model.sample_rate * seconds))) * 0.1
    model.enhance(sample[:, : model.sample_rate])
    started = time.perf_counter()
    model.enhance(sample)
    return (time.perf_counter() - started) / seconds
