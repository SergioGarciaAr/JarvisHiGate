"""Builds an MLX Chatterbox model with the official Latin American Spanish
finetune: base MLX artifact (mlx-community/chatterbox-multilingual-v3) with
its T3 weights replaced by t3_es_mx_latam, mapped with the same sanitize()
as mlx-audio's official converter.

S3Gen is kept from the official MLX artifact. Replacing it with s3gen_v3
passed the strict key/shape validation but produced saturated audio (median
-3.7 dBFS, peaks 0.99) versus -16.7 dBFS from the original PyTorch model:
mlx-audio's S3Token2Wav does not implement what the v3 decoder expects.
Measured on 2026-10-19 (iso-A/iso-B comparison in the bench report)."""
import json, shutil, sys
from pathlib import Path

import mlx.core as mx
from huggingface_hub import snapshot_download
from mlx_audio.tts.models.chatterbox.config import T3Config
from mlx_audio.tts.models.chatterbox.t3 import T3

t3_src, out = Path(sys.argv[1]), Path(sys.argv[2])
base = Path(snapshot_download("mlx-community/chatterbox-multilingual-v3"))
out.mkdir(parents=True, exist_ok=True)
weights = mx.load(str(base / "model.safetensors"))
dtype = next(v.dtype for k, v in weights.items() if k.startswith("t3."))
print("base:", len(weights), "tensores · dtype T3", dtype)

t3_new = T3(T3Config.multilingual()).sanitize(mx.load(str(t3_src)))
old_t3 = {k for k in weights if k.startswith("t3.")}
new_t3 = {f"t3.{k}" for k in t3_new}
if old_t3 != new_t3:
    raise SystemExit(f"T3 no coincide: faltan {sorted(old_t3 - new_t3)[:5]} · sobran {sorted(new_t3 - old_t3)[:5]}")
for k, v in t3_new.items():
    weights[f"t3.{k}"] = v.astype(dtype)

mx.save_safetensors(str(out / "model.safetensors"), weights)
for f in ["tokenizer.json", "Cangjie5_TC.json", "config.json"]:
    shutil.copy(base / f, out / f)
cfg = json.loads((out / "config.json").read_text())
cfg["source"] = "t3_es_mx_latam (ResembleAI/Chatterbox-Multilingual-es-mx-latam, MIT) sobre mlx-community/chatterbox-multilingual-v3 (S3Gen oficial MLX)"
(out / "config.json").write_text(json.dumps(cfg, indent=2, ensure_ascii=False))
print("listo:", out, f"{(out / 'model.safetensors').stat().st_size/1e9:.2f} GB")
