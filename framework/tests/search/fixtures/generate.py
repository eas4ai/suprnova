"""Regenerate tiny graphs that expose batching and external-tensor regressions."""
from pathlib import Path
import onnx
from onnx import helper as h, TensorProto as T
p=Path(__file__).parent
# Quantization range deliberately depends on all token IDs in the batch.
nodes=[h.make_node('Cast',['input_ids'],['tokens'],to=T.FLOAT),h.make_node('DynamicQuantizeLinear',['tokens'],['quantized','scale','zero']),h.make_node('DequantizeLinear',['quantized','scale','zero'],['dequantized']),h.make_node('Unsqueeze',['dequantized','axes'],['first']),h.make_node('Mul',['dequantized','constant'],['second_scalar']),h.make_node('Add',['second_scalar','one'],['second_shifted']),h.make_node('Unsqueeze',['second_shifted','axes'],['second']),h.make_node('Concat',['first','second'],['custom_output'],axis=2)]
graph=h.make_graph(nodes,'batch-sensitive-fixture',[h.make_tensor_value_info('input_ids',T.INT64,['batch','tokens']),h.make_tensor_value_info('attention_mask',T.INT64,['batch','tokens'])],[h.make_tensor_value_info('custom_output',T.FLOAT,['batch','tokens',2])],[h.make_tensor('axes',T.INT64,[1],[2]),h.make_tensor('constant',T.FLOAT,[],[0.25]),h.make_tensor('one',T.FLOAT,[],[1.0])])
model=h.make_model(graph,opset_imports=[h.make_opsetid('',13)],ir_version=8)
onnx.checker.check_model(model)
onnx.save(model,p/'text.onnx')

import numpy as np
(p/'sparse.onnx_data').unlink(missing_ok=True)
weight=onnx.numpy_helper.from_array(np.array([1.0],dtype=np.float32),name='weights')
nodes=[h.make_node('Cast',['input_ids'],['ids_float'],to=T.FLOAT),h.make_node('Unsqueeze',['ids_float','axes'],['ids3']),h.make_node('Add',['ids3','weights'],['sparse_vecs']),h.make_node('ReduceMean',['ids_float'],['dense_vecs'],axes=[1],keepdims=1),h.make_node('Slice',['sparse_vecs','starts','ends','slice_axes'],['colbert_vecs'])]
graph=h.make_graph(nodes,'external-joint-sparse-fixture',[h.make_tensor_value_info('input_ids',T.INT64,['batch','tokens']),h.make_tensor_value_info('attention_mask',T.INT64,['batch','tokens'])],[h.make_tensor_value_info('dense_vecs',T.FLOAT,['batch',1]),h.make_tensor_value_info('sparse_vecs',T.FLOAT,['batch','tokens',1]),h.make_tensor_value_info('colbert_vecs',T.FLOAT,['batch','colbert_tokens',1])],[h.make_tensor('axes',T.INT64,[1],[2]),h.make_tensor('starts',T.INT64,[1],[1]),h.make_tensor('ends',T.INT64,[1],[2147483647]),h.make_tensor('slice_axes',T.INT64,[1],[1]),weight])
model=h.make_model(graph,opset_imports=[h.make_opsetid('',13)],ir_version=8)
onnx.save_model(model,p/'sparse.onnx',save_as_external_data=True,all_tensors_to_one_file=True,location='sparse.onnx_data',size_threshold=0)
