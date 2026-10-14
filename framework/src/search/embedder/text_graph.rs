//! The multimodal export requires empty feature tensors for a text-only invocation.
use crate::FrameworkError;
use onnx_protobuf::{Message, ModelProto, TensorProto};

pub(super) fn embeddinggemma_text_graph(bytes: &[u8]) -> Result<Vec<u8>, FrameworkError> {
    let mut model = ModelProto::parse_from_bytes(bytes)
        .map_err(|e| FrameworkError::internal(format!("EmbeddingGemma 2 ONNX graph: {e}")))?;
    let graph = model
        .graph
        .as_mut()
        .ok_or_else(|| FrameworkError::param("EmbeddingGemma 2 ONNX graph is missing"))?;
    for name in ["image_features", "video_features", "audio_features"] {
        let input = graph
            .input
            .iter()
            .find(|input| input.name == name)
            .ok_or_else(|| {
                FrameworkError::param(format!(
                    "EmbeddingGemma 2 ONNX graph requires the declared {name} input"
                ))
            })?;
        let dimensions = &input.type_.tensor_type().shape.dim;
        if input.type_.tensor_type().elem_type != 1
            || dimensions.len() != 2
            || dimensions[1].dim_value() != 512
        {
            return Err(FrameworkError::param(format!(
                "EmbeddingGemma 2 ONNX graph {name} must contain float32 feature rows of width 512"
            )));
        }
        let mut tensor = TensorProto::new();
        tensor.name = name.into();
        tensor.data_type = 1;
        tensor.dims = vec![0, 512];
        graph.initializer.push(tensor);
        graph.input.retain(|input| input.name != name);
    }
    model
        .write_to_bytes()
        .map_err(|e| FrameworkError::internal(format!("EmbeddingGemma 2 text ONNX graph: {e}")))
}
