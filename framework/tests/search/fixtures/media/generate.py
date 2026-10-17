"""Generate original processor fixtures with transformers 5.20.1 on CPU.

Run in an external uv venv with torch CPU, pillow, numpy and soundfile.
Outputs use little endian float32 and int64, gzip with a fixed timestamp.
"""
import gzip
import json
from pathlib import Path
import urllib.request
import subprocess
import imageio_ffmpeg

import numpy as np
from PIL import Image, ImageDraw
import soundfile as sf
import transformers
from transformers import Gemma4AudioFeatureExtractor, Gemma4ImageProcessorPil, EmbeddingGemma2VideoProcessor

ROOT = Path(__file__).parent
assert transformers.__version__ == "5.20.1"
config = json.loads(urllib.request.urlopen(
    "https://huggingface.co/onnx-community/embeddinggemma-2-ONNX/raw/daa72c51243991dfcaf9f9137d2c573d8f7790c0/processor_config.json"
).read())
(ROOT / "processor_config.json").write_text(json.dumps(config, indent=2) + "\n")
manifest = {"transformers": transformers.__version__, "model_revision": "daa72c51243991dfcaf9f9137d2c573d8f7790c0", "fixtures": {}}


def write(name, values, dtype, tolerance):
    array = np.asarray(values, dtype=dtype)
    (ROOT / (name + ".gz")).write_bytes(gzip.compress(array.tobytes(), mtime=0))
    manifest["fixtures"][name] = {"shape": list(array.shape), "dtype": dtype,
                                  "absolute_tolerance": tolerance}


image = Image.new("RGB", (64, 48), "white")
draw = ImageDraw.Draw(image)
draw.rectangle((4, 5, 25, 34), fill="red")
draw.ellipse((32, 10, 58, 37), fill="blue")
image.save(ROOT / "shapes.png")
Image.new("RGB", (64,48), "green").save(ROOT / "unrelated.png")
for name, frequency, shift in [("tones_shifted", 440, 0.08), ("tones_other", 1700, 0)]:
    time = np.arange(24000) / 16000 + shift
    waveform = 0.3 * np.sin(2 * np.pi * frequency * time) + 0.2 * np.sin(2 * np.pi * frequency * 2 * time)
    sf.write(ROOT / (name + ".wav"), waveform, 16000, subtype="PCM_16")
subprocess.run([imageio_ffmpeg.get_ffmpeg_exe(), "-y", "-loglevel", "error",
                "-loop", "1", "-framerate", "1", "-i", str(ROOT / "shapes.png"),
                "-t", "3", "-fflags", "+bitexact", "-flags:v", "+bitexact", "-c:v", "ffv1", str(ROOT / "shapes.mkv")], check=True)
for budget, name in [(280, "image"), (140, "video_frame")]:
    processor = Gemma4ImageProcessorPil(**(config["image_processor"] | {"max_soft_tokens": budget}))
    output = processor(images=image, return_tensors="np")
    write(name + "_pixels", output["pixel_values"], "<f4", 1e-7)
    write(name + "_positions", output["image_position_ids"], "<i8", 0)
    write(name + "_tokens", output["num_soft_tokens_per_image"], "<i8", 0)
video = np.stack([np.asarray(image)] * 3)
video_output = EmbeddingGemma2VideoProcessor(**config["video_processor"])(
    videos=[video], do_sample_frames=False, return_tensors="np")
# Torch and Pillow bicubic round uint8 channels independently. One channel
# step is the recorded cross-backend tolerance for decoded video frames.
write("video_pixels", video_output["pixel_values_videos"], "<f4", 1 / 255 + 1e-7)
write("video_positions", video_output["video_position_ids"], "<i8", 0)
for rate in [16000, 44100]:
    time = np.arange(int(rate * 1.5)) / rate
    waveform = (0.3 * np.sin(2 * np.pi * 440 * time) + 0.2 * np.sin(2 * np.pi * 880 * time))
    name = f"tones_{rate}"
    sf.write(ROOT / (name + ".wav"), waveform, rate, subtype="PCM_16")
    waveform, _ = sf.read(ROOT / (name + ".wav"), dtype="float32")
    if rate != 16000:
        # Resampling is outside the feature extractor. Record the explicit
        # bandlimited reference so Rust must match the waveform as well.
        import torch
        import torchaudio.functional as AF
        waveform = AF.resample(torch.from_numpy(waveform), rate, 16000).numpy()
    output = Gemma4AudioFeatureExtractor(**config["feature_extractor"])([waveform], return_tensors="np")
    write(name + "_features", output["input_features"], "<f4", 0.0002)
    write(name + "_mask", output["input_features_mask"], "u1", 0)
(ROOT / "reference.json").write_text(json.dumps(manifest, indent=2) + "\n")
