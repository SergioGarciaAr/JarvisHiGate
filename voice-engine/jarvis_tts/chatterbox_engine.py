"""Chatterbox Multilingual V3 with the official Latin American Spanish
finetune (ResembleAI/Chatterbox-Multilingual-es-mx-latam, MIT).

The finetune ships `t3_es_mx_latam.safetensors` and `s3gen_v3.pt`. The
upstream `from_local` loads `s3gen.pt` strictly, but the V3 decoder omits two
deterministic tokenizer buffers that the model itself declares as ignorable
(`S3Gen.ignore_state_dict_missing`). We load it non-strictly and verify that
nothing else is missing.
"""
from pathlib import Path

import torch
from chatterbox.models.s3gen import S3Gen
from chatterbox.models.t3 import T3
from chatterbox.models.t3.modules.t3_config import T3Config
from chatterbox.models.tokenizers import MTLTokenizer
from chatterbox.models.voice_encoder import VoiceEncoder
from chatterbox.mtl_tts import ChatterboxMultilingualTTS, Conditionals
from safetensors.torch import load_file

T3_FILE = "t3_es_mx_latam.safetensors"


def load(ckpt_dir: Path, device: str = "mps") -> ChatterboxMultilingualTTS:
    ckpt_dir = Path(ckpt_dir)
    cpu = torch.device("cpu")
    ve = VoiceEncoder()
    ve.load_state_dict(torch.load(ckpt_dir / "ve.pt", map_location=cpu, weights_only=True))
    ve.to(device).eval()

    t3 = T3(T3Config.multilingual())
    state = load_file(ckpt_dir / T3_FILE)
    if "model" in state:
        state = state["model"][0]
    t3.load_state_dict(state)
    t3.to(device).eval()

    s3gen = S3Gen()
    result = s3gen.load_state_dict(torch.load(ckpt_dir / "s3gen.pt", map_location=cpu, weights_only=True), strict=False)
    unexpected_missing = set(result.missing_keys) - set(S3Gen.ignore_state_dict_missing)
    if unexpected_missing or result.unexpected_keys:
        raise RuntimeError(f"s3gen_v3 no coincide: faltan {sorted(unexpected_missing)}, sobran {result.unexpected_keys}")
    s3gen.to(device).eval()

    tokenizer = MTLTokenizer(str(ckpt_dir / "grapheme_mtl_merged_expanded_v1.json"))
    conds = Conditionals.load(ckpt_dir / "conds.pt", map_location=cpu).to(device)
    return ChatterboxMultilingualTTS(t3, s3gen, ve, tokenizer, device, conds=conds)
