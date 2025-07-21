use std::sync::Arc;

use super::{BufferInitContent, FragmentShaderData, FragmentShaderKey};

/// The palette shader, computing texture coordinates from an input color.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShaderData {
    pub mix: [f32; 4],
    /// How to determine the color to mix from the foreground (dot product).
    pub color: [f32; 4],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shader {
    pub data: ShaderData,
    pub spirv: Arc<[u8]>,
}

impl FragmentShaderData for Shader {
    fn key(&self) -> Option<FragmentShaderKey> {
        Some(FragmentShaderKey::Inject)
    }

    fn spirv_source(&self) -> Arc<[u8]> {
        self.spirv.clone()
    }

    fn binary_data(&self, buffer: &mut Vec<u8>) -> Option<BufferInitContent> {
        let data = [self.data.mix, self.data.color];
        Some(BufferInitContent::new(buffer, &data))
    }

    fn num_args(&self) -> u32 {
        2
    }
}
