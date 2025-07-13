use std::sync::Arc;

use super::{BufferInitContent, Direction, FragmentShaderData, FragmentShaderKey};
use crate::buffer::Whitepoint;
use crate::color_matrix::RowMatrix;

/// The palette shader, computing texture coordinates from an input color.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Shader {
    pub matrix: RowMatrix,
    pub whitepoint: Whitepoint,
    pub direction: Coding,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Coding {
    Encode { spirv: Arc<[u8]> },
    Decode { spirv: Arc<[u8]> },
}

impl FragmentShaderData for Shader {
    /// The unique key identifying this shader module.
    fn key(&self) -> Option<FragmentShaderKey> {
        Some(FragmentShaderKey::Srlab2Transform(match self.direction {
            Coding::Encode { .. } => Direction::Encode,
            Coding::Decode { .. } => Direction::Decode,
        }))
    }

    /// The SPIR-V shader source code.
    fn spirv_source(&self) -> Arc<[u8]> {
        match &self.direction {
            Coding::Encode { spirv } | Coding::Decode { spirv } => spirv.clone(),
        }
    }

    /// Encode the shader's data into the buffer, returning the descriptor to that.
    fn binary_data(&self, buffer: &mut Vec<u8>) -> Option<BufferInitContent> {
        let data = self.matrix.into_mat3x3_std140();
        let wp = self.whitepoint.to_xyz();

        let mut content = BufferInitContent::builder(buffer);
        content.extend_from_pods(&[wp]);
        content.align_by_exponent(4);
        content.extend_from_pods(&[data]);
        Some(content.build())
    }

    fn num_args(&self) -> u32 {
        1
    }
}
