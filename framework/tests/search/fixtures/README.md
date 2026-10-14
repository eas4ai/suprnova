# Tiny ONNX fixtures

`text.onnx` applies dynamic quantization to token IDs before producing two token features. Mixing low and high IDs changes the quantization range, so the test catches batch-dependent embeddings rather than checking a batch-size field.

`sparse.onnx` returns the three tensors fastembed's joint BGE-M3 loader expects. Its scalar initializer lives in `sparse.onnx_data`, so loading it exercises external tensor resolution. The tokenizer and metadata are deliberately small and support both fixtures.

Regenerate the graphs with `uv run --with onnx python framework/tests/search/fixtures/generate.py`. They use ONNX IR 8 and opset 13. These fixtures contain no pretrained weights and need no model download.
