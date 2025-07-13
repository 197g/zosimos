use std::sync::Arc;

use super::{BufferInitContent, Direction, FragmentShaderData, FragmentShaderKey};
use crate::color_matrix::RowMatrix;

/// The palette shader, computing texture coordinates from an input color.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Shader {
    pub xyz_transform: RowMatrix,
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
        Some(FragmentShaderKey::OklabTransform(match self.direction {
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
        let data = self.xyz_transform.into_mat3x3_std140();
        Some(BufferInitContent::new(buffer, &data))
    }

    fn num_args(&self) -> u32 {
        1
    }
}
