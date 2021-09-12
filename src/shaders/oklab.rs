use std::borrow::Cow;

use super::{BufferInitContent, FragmentShaderData, FragmentShaderKey};
use crate::buffer::RowMatrix;

/// a linear transformation on rgb color.
pub const SHADER_ENCODE: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/spirv/oklab_encode.frag.v"));
pub const SHADER_DECODE: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/spirv/oklab_decode.frag.v"));

/// The palette shader, computing texture coordinates from an input color.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Shader {
    matrix: RowMatrix,
    direction: Direction,
}

#[derive(Clone, Debug, PartialEq)]
enum Direction {
    Encode,
    Decode,
}

impl Shader {
    pub fn with_encode(matrix: RowMatrix) -> Self {
        Shader {
            matrix,
            direction: Direction::Encode,
        }
    }

    pub fn with_decode(matrix: RowMatrix) -> Self {
        Shader {
            matrix,
            direction: Direction::Decode,
        }
    }
}

impl FragmentShaderData for Shader {
    /// The unique key identifying this shader module.
    fn key(&self) -> Option<FragmentShaderKey> {
        Some(FragmentShaderKey::OklabTransform(true))
    }

    /// The SPIR-V shader source code.
    fn spirv_source(&self) -> Cow<'static, [u8]> {
        Cow::Borrowed(match self.direction {
            Direction::Encode => SHADER_ENCODE,
            Direction::Decode => SHADER_DECODE,
        })
    }

    /// Encode the shader's data into the buffer, returning the descriptor to that.
    fn binary_data(&self, buffer: &mut Vec<u8>) -> Option<BufferInitContent> {
        let data = self.matrix.into_mat3x3_std140();

        Some(BufferInitContent::new(buffer, &data))
    }

    fn num_args(&self) -> u32 {
        1
    }
}
