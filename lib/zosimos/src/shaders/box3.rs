use std::sync::Arc;

use super::{BufferInitContent, FragmentShaderData, FragmentShaderKey};
use crate::color_matrix::RowMatrix;

/// The palette shader, computing texture coordinates from an input color.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Shader {
    pub matrix: RowMatrix,
    pub spirv: Arc<[u8]>,
}

impl FragmentShaderData for Shader {
    /// The unique key identifying this shader module.
    fn key(&self) -> Option<FragmentShaderKey> {
        Some(FragmentShaderKey::Box3)
    }

    /// The SPIR-V shader source code.
    fn spirv_source(&self) -> Arc<[u8]> {
        self.spirv.clone()
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
