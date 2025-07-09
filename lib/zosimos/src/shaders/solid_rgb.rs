use crate::program::BufferInitContent;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub struct ShaderData([f32; 4]);

#[derive(Debug, Clone, PartialEq)]
pub struct Shader {
    pub data: ShaderData,
    pub spirv: Arc<[u8]>,
}

impl super::FragmentShaderData for Shader {
    fn key(&self) -> Option<super::FragmentShaderKey> {
        None
    }

    fn spirv_source(&self) -> Arc<[u8]> {
        self.spirv.clone()
    }

    fn binary_data(&self, buffer: &mut Vec<u8>) -> Option<BufferInitContent> {
        Some(BufferInitContent::new(buffer, &self.data.0))
    }

    fn num_args(&self) -> u32 {
        0
    }
}

impl From<[f32; 4]> for ShaderData {
    fn from(value: [f32; 4]) -> Self {
        ShaderData(value)
    }
}
