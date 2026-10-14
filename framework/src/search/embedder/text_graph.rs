//! The multimodal export requires empty feature tensors for a text-only invocation.
use crate::FrameworkError;

const MEDIA_INPUTS: [&str; 3] = ["image_features", "video_features", "audio_features"];

pub(super) fn embeddinggemma_text_graph(bytes: &[u8]) -> Result<Vec<u8>, FrameworkError> {
    let mut remaining = bytes;
    let mut output = Vec::with_capacity(bytes.len());
    let mut found_graph = false;
    while let Some(record) = read_record(&mut remaining)? {
        if record.number == 7 {
            let graph = text_graph(record.bytes()?)?;
            write_bytes(7, &graph, &mut output);
            found_graph = true;
        } else {
            output.extend_from_slice(record.raw);
        }
    }
    if !found_graph {
        return Err(FrameworkError::param(
            "EmbeddingGemma 2 ONNX graph is missing",
        ));
    }
    Ok(output)
}

fn text_graph(bytes: &[u8]) -> Result<Vec<u8>, FrameworkError> {
    let mut remaining = bytes;
    let mut output = Vec::with_capacity(bytes.len());
    let mut found = [false; 3];
    while let Some(record) = read_record(&mut remaining)? {
        if record.number == 11 {
            let input = record.bytes()?;
            let name = length_field(input, 1)?.unwrap_or_default();
            let name =
                std::str::from_utf8(name).map_err(|_| wire_error("input name is not UTF-8"))?;
            if let Some(index) = MEDIA_INPUTS.iter().position(|media| *media == name) {
                validate_media_input(input, name)?;
                found[index] = true;
                continue;
            }
        }
        output.extend_from_slice(record.raw);
    }
    for (name, found) in MEDIA_INPUTS.into_iter().zip(found) {
        if !found {
            return Err(FrameworkError::param(format!(
                "EmbeddingGemma 2 ONNX graph requires the declared {name} input"
            )));
        }
        // TensorProto: unpacked dims [0, 512], FLOAT data_type, and name.
        let mut tensor = vec![0x08, 0, 0x08, 0x80, 0x04, 0x10, 1];
        write_bytes(8, name.as_bytes(), &mut tensor);
        write_bytes(5, &tensor, &mut output);
    }
    Ok(output)
}

fn validate_media_input(input: &[u8], name: &str) -> Result<(), FrameworkError> {
    let value_type = length_field(input, 2)?.unwrap_or_default();
    // TypeProto's tensor field is a oneof with sequence, map, sparse and optional.
    let tensor = read_field(value_type, 1, 2, &[4, 5, 8, 9])?
        .map(|record| record.payload)
        .unwrap_or_default();
    let elem_type = varint_field(tensor, 1)?;
    let mut shape = length_field(tensor, 2)?.unwrap_or_default();
    let mut dimensions = 0_usize;
    let mut width = 0;
    while let Some(record) = read_record(&mut shape)? {
        if record.number == 1 {
            dimensions += 1;
            // A later symbolic dim_param clears the preceding numeric dim_value.
            width = read_field(record.bytes()?, 1, 0, &[2])?
                .map(|record| record.varint())
                .transpose()?
                .unwrap_or_default();
        }
    }
    if elem_type != 1 || dimensions != 2 || width != 512 {
        return Err(FrameworkError::param(format!(
            "EmbeddingGemma 2 ONNX graph {name} must contain float32 feature rows of width 512"
        )));
    }
    Ok(())
}

// Only the fixed Model -> Graph -> ValueInfo -> Type -> Tensor -> Shape -> Dim
// path is decoded. Unknown length-delimited payloads stay opaque, with no recursion.
struct Record<'a> {
    number: u64,
    wire_type: u8,
    raw: &'a [u8],
    payload: &'a [u8],
}

impl<'a> Record<'a> {
    fn bytes(&self) -> Result<&'a [u8], FrameworkError> {
        if self.wire_type != 2 {
            return Err(wire_error("expected a length-delimited field"));
        }
        Ok(self.payload)
    }

    fn varint(&self) -> Result<u64, FrameworkError> {
        if self.wire_type != 0 {
            return Err(wire_error("expected a varint field"));
        }
        let mut payload = self.payload;
        read_varint(&mut payload)
    }
}

fn read_record<'a>(remaining: &mut &'a [u8]) -> Result<Option<Record<'a>>, FrameworkError> {
    if remaining.is_empty() {
        return Ok(None);
    }
    let start = *remaining;
    let tag = read_varint(remaining)?;
    let number = tag >> 3;
    if number == 0 || number > 0x1fff_ffff {
        return Err(wire_error("invalid field number"));
    }
    let wire_type = (tag & 7) as u8;
    let payload = match wire_type {
        0 => {
            let start = *remaining;
            read_varint(remaining)?;
            &start[..start.len() - remaining.len()]
        }
        1 => take_bytes(remaining, 8)?,
        2 => {
            let length = usize::try_from(read_varint(remaining)?)
                .map_err(|_| wire_error("field length exceeds addressable memory"))?;
            take_bytes(remaining, length)?
        }
        5 => take_bytes(remaining, 4)?,
        _ => return Err(wire_error("unsupported wire type")),
    };
    Ok(Some(Record {
        number,
        wire_type,
        raw: &start[..start.len() - remaining.len()],
        payload,
    }))
}

fn take_bytes<'a>(remaining: &mut &'a [u8], length: usize) -> Result<&'a [u8], FrameworkError> {
    if length > remaining.len() {
        return Err(wire_error("truncated field payload"));
    }
    let (payload, rest) = remaining.split_at(length);
    *remaining = rest;
    Ok(payload)
}

fn read_varint(remaining: &mut &[u8]) -> Result<u64, FrameworkError> {
    let mut value = 0;
    for shift in (0..70).step_by(7) {
        let (&byte, rest) = remaining
            .split_first()
            .ok_or_else(|| wire_error("truncated varint"))?;
        *remaining = rest;
        if shift == 63 && byte > 1 {
            return Err(wire_error("varint exceeds 64 bits"));
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(wire_error("varint exceeds 64 bits"))
}

fn read_field<'a>(
    mut bytes: &'a [u8],
    number: u64,
    wire_type: u8,
    alternatives: &[u64],
) -> Result<Option<Record<'a>>, FrameworkError> {
    let mut value = None;
    while let Some(record) = read_record(&mut bytes)? {
        if record.number == number {
            if record.wire_type != wire_type {
                return Err(wire_error("unexpected field wire type"));
            }
            value = Some(record);
        } else if alternatives.contains(&record.number) {
            record.bytes()?;
            value = None;
        }
    }
    Ok(value)
}

fn length_field(bytes: &[u8], number: u64) -> Result<Option<&[u8]>, FrameworkError> {
    Ok(read_field(bytes, number, 2, &[])?.map(|record| record.payload))
}

fn varint_field(bytes: &[u8], number: u64) -> Result<u64, FrameworkError> {
    Ok(read_field(bytes, number, 0, &[])?
        .map(|record| record.varint())
        .transpose()?
        .unwrap_or_default())
}

fn write_varint(mut value: u64, output: &mut Vec<u8>) {
    while value >= 0x80 {
        output.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    output.push(value as u8);
}

fn write_bytes(number: u64, payload: &[u8], output: &mut Vec<u8>) {
    write_varint((number << 3) | 2, output);
    write_varint(payload.len() as u64, output);
    output.extend_from_slice(payload);
}

fn wire_error(message: &str) -> FrameworkError {
    FrameworkError::internal(format!("EmbeddingGemma 2 ONNX graph: {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_codec_matches_wire_values() {
        for (value, bytes) in [
            (0, vec![0]),
            (127, vec![127]),
            (128, vec![0x80, 1]),
            (512, vec![0x80, 4]),
            (16_384, vec![0x80, 0x80, 1]),
            (
                u64::MAX,
                vec![0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 1],
            ),
        ] {
            let mut encoded = Vec::new();
            write_varint(value, &mut encoded);
            assert_eq!(encoded, bytes);
            let mut remaining = bytes.as_slice();
            assert_eq!(read_varint(&mut remaining).unwrap(), value);
            assert!(remaining.is_empty());
        }
        let mut non_minimal = [0x81, 0].as_slice();
        assert_eq!(read_varint(&mut non_minimal).unwrap(), 1);
    }

    #[test]
    fn varint_codec_rejects_truncation_and_overflow() {
        for bytes in [vec![], vec![0x80], vec![0xff; 10], vec![0x80; 11]] {
            assert!(read_varint(&mut bytes.as_slice()).is_err());
        }
        let mut overflow = vec![0xff; 9];
        overflow.push(2);
        assert!(read_varint(&mut overflow.as_slice()).is_err());
    }

    #[test]
    fn length_codec_handles_multibyte_lengths_and_tags() {
        for length in [0, 127, 128, 512] {
            let payload = vec![0x42; length];
            let mut encoded = Vec::new();
            write_bytes(101, &payload, &mut encoded);
            assert_eq!(&encoded[..2], &[0xaa, 0x06]);
            if length == 512 {
                assert_eq!(&encoded[2..4], &[0x80, 0x04]);
            }
            let mut remaining = encoded.as_slice();
            let record = read_record(&mut remaining).unwrap().unwrap();
            assert_eq!(record.number, 101);
            assert_eq!(record.bytes().unwrap(), payload);
            assert_eq!(record.raw, encoded);
            assert!(remaining.is_empty());
        }
        for bytes in [
            vec![0x0a],
            vec![0x0a, 0x80],
            vec![0x0a, 2, 1],
            vec![
                0x0a, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 1,
            ],
        ] {
            assert!(read_record(&mut bytes.as_slice()).is_err());
        }
    }

    // Small, hand-encoded messages keep these fixtures independent of the codec.
    fn message(tag: u8, payload: &[u8]) -> Vec<u8> {
        let length = u8::try_from(payload.len()).unwrap();
        assert!(length < 128);
        let mut bytes = vec![tag, length];
        bytes.extend_from_slice(payload);
        bytes
    }

    fn input(name: &str, width: &[u8]) -> Vec<u8> {
        let mut shape = vec![0x0a, 2, 0x08, 0];
        let mut dim = vec![0x08];
        dim.extend_from_slice(width);
        shape.extend(message(0x0a, &dim));
        let mut tensor = vec![0x08, 1];
        tensor.extend(message(0x12, &shape));
        let mut value = message(0x0a, name.as_bytes());
        value.extend(message(0x12, &message(0x0a, &tensor)));
        message(0x5a, &value)
    }

    fn model(names: &[&str], width: &[u8]) -> Vec<u8> {
        let graph: Vec<u8> = names.iter().flat_map(|name| input(name, width)).collect();
        message(0x3a, &graph)
    }

    #[test]
    fn unrelated_top_level_records_survive_byte_for_byte() {
        // Field 100 contains a deliberately non-minimal encoding of the value 1.
        let extra = [
            0xa0, 0x06, 0x81, 0x00, // Varint.
            0xa9, 0x06, 1, 2, 3, 4, 5, 6, 7, 8, // Fixed64.
            0xb2, 0x06, 0x82, 0, 0xff, 0xff, // Opaque bytes, non-minimal length.
            0xbd, 0x06, 1, 2, 3, 4, // Fixed32.
        ];
        let mut bytes = extra.to_vec();
        bytes.extend(model(
            &["image_features", "video_features", "audio_features"],
            &[0x80, 0x04],
        ));
        bytes.extend_from_slice(&extra);
        let rewritten = embeddinggemma_text_graph(&bytes).unwrap();
        assert!(rewritten.starts_with(&extra));
        assert!(rewritten.ends_with(&extra));
    }

    #[test]
    fn graph_records_survive_and_media_inputs_become_empty_initializers() {
        let preserved = [
            0x5a, 7, 0x0a, 5, b't', b'o', b'k', b'e', b'n', // Text input.
            0x2a, 0, // Existing initializer.
            0xa0, 0x06, 0x81, 0, // Unknown field.
        ];
        let mut graph = preserved.to_vec();
        for name in MEDIA_INPUTS {
            graph.extend(input(name, &[0x80, 4]));
        }
        let rewritten = embeddinggemma_text_graph(&message(0x3a, &graph)).unwrap();
        let mut graph = length_field(&rewritten, 7).unwrap().unwrap();
        assert!(graph.starts_with(&preserved));
        graph = &graph[preserved.len()..];
        for name in MEDIA_INPUTS {
            let record = read_record(&mut graph).unwrap().unwrap();
            assert_eq!(record.number, 5);
            let tensor = record.bytes().unwrap();
            assert_eq!(&tensor[..7], &[0x08, 0, 0x08, 0x80, 4, 0x10, 1]);
            assert_eq!(length_field(tensor, 8).unwrap().unwrap(), name.as_bytes());
        }
        assert!(graph.is_empty());
    }

    #[test]
    fn missing_input_keeps_the_existing_error() {
        let error =
            embeddinggemma_text_graph(&model(&["image_features", "video_features"], &[0x80, 0x04]))
                .unwrap_err();
        assert!(matches!(error, FrameworkError::ParamError { param_name }
            if param_name == "EmbeddingGemma 2 ONNX graph requires the declared audio_features input"));
    }

    #[test]
    fn wrong_width_keeps_the_existing_error() {
        let error = embeddinggemma_text_graph(&model(
            &["image_features", "video_features", "audio_features"],
            &[0xff, 0x03],
        ))
        .unwrap_err();
        assert!(matches!(error, FrameworkError::ParamError { param_name }
            if param_name == "EmbeddingGemma 2 ONNX graph image_features must contain float32 feature rows of width 512"));
    }

    #[test]
    fn media_inputs_require_float32_and_exactly_two_dimensions() {
        for tensor in [
            vec![], // Missing element type and shape.
            vec![0x08, 2, 0x12, 9, 0x0a, 2, 0x08, 0, 0x0a, 3, 0x08, 0x80, 4],
            vec![0x08, 1],                                  // Missing shape.
            vec![0x08, 1, 0x12, 5, 0x0a, 3, 0x08, 0x80, 4], // One dimension.
            vec![
                0x08, 1, 0x12, 11, 0x0a, 0, 0x0a, 2, 0x08, 0, 0x0a, 3, 0x08, 0x80, 4,
            ],
            vec![0x08, 1, 0x12, 9, 0x0a, 2, 0x08, 0, 0x0a, 3, 0x12, 1, b'n'],
        ] {
            let mut value = message(0x0a, b"image_features");
            value.extend(message(0x12, &message(0x0a, &tensor)));
            let bytes = message(0x3a, &message(0x5a, &value));
            assert!(
                matches!(embeddinggemma_text_graph(&bytes), Err(FrameworkError::ParamError { param_name })
                if param_name == "EmbeddingGemma 2 ONNX graph image_features must contain float32 feature rows of width 512")
            );
        }
    }

    #[test]
    fn later_oneof_alternatives_clear_the_tensor_type_or_width() {
        let valid_tensor = [0x08, 1, 0x12, 9, 0x0a, 2, 0x08, 0, 0x0a, 3, 0x08, 0x80, 4];
        for alternative in [0x22, 0x2a, 0x42, 0x4a] {
            let mut value_type = message(0x0a, &valid_tensor);
            value_type.extend([alternative, 0]);
            let value = message(0x12, &value_type);
            assert!(
                matches!(validate_media_input(&value, "image_features"), Err(FrameworkError::ParamError { param_name })
                if param_name == "EmbeddingGemma 2 ONNX graph image_features must contain float32 feature rows of width 512")
            );

            let mut value_type = vec![alternative, 0];
            value_type.extend(message(0x0a, &valid_tensor));
            assert!(validate_media_input(&message(0x12, &value_type), "image_features").is_ok());
        }

        let symbolic_width = [
            0x08, 1, 0x12, 12, 0x0a, 2, 0x08, 0, 0x0a, 6, 0x08, 0x80, 4, 0x12, 1, b'N',
        ];
        let value = message(0x12, &message(0x0a, &symbolic_width));
        assert!(
            matches!(validate_media_input(&value, "image_features"), Err(FrameworkError::ParamError { param_name })
            if param_name == "EmbeddingGemma 2 ONNX graph image_features must contain float32 feature rows of width 512")
        );

        let concrete_width = [
            0x08, 1, 0x12, 12, 0x0a, 2, 0x08, 0, 0x0a, 6, 0x12, 1, b'N', 0x08, 0x80, 4,
        ];
        let value = message(0x12, &message(0x0a, &concrete_width));
        assert!(validate_media_input(&value, "image_features").is_ok());
    }

    #[test]
    fn truncated_records_return_a_typed_error() {
        let bytes = model(
            &["image_features", "video_features", "audio_features"],
            &[0x80, 0x04],
        );
        for length in 1..bytes.len() {
            assert!(matches!(
                embeddinggemma_text_graph(&bytes[..length]),
                Err(FrameworkError::Internal { .. })
            ));
        }
    }

    #[test]
    fn malformed_records_return_a_typed_error() {
        for bytes in [
            vec![0],                               // Field zero.
            vec![0x80, 0x80, 0x80, 0x80, 0x10, 0], // Field number above 2^29 - 1.
            vec![0x0b],
            vec![0x0c],
            vec![0x0e],
            vec![0x0f], // Unsupported wire types.
            vec![0x09, 0],
            vec![0x0d, 0], // Truncated fixed-width values.
            vec![0x38, 0], // Graph with the wrong wire type.
            message(0x3a, &[0x5a, 4, 0x0a, 2, 0xff, 0xff]), // Invalid name.
            message(
                0x3a,
                &message(
                    0x5a,
                    &[
                        0x0a, 14, b'i', b'm', b'a', b'g', b'e', b'_', b'f', b'e', b'a', b't', b'u',
                        b'r', b'e', b's', 0x12, 3, 0x0a, 2, 0x08, // Truncated nested type.
                    ],
                ),
            ),
        ] {
            assert!(matches!(
                embeddinggemma_text_graph(&bytes),
                Err(FrameworkError::Internal { .. })
            ));
        }
    }

    #[test]
    fn missing_graph_keeps_the_existing_error() {
        assert!(
            matches!(embeddinggemma_text_graph(&[0x08, 1]), Err(FrameworkError::ParamError { param_name })
            if param_name == "EmbeddingGemma 2 ONNX graph is missing")
        );
    }
}
