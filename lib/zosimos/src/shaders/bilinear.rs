use std::sync::Arc;

use super::{BufferInitContent, FragmentShaderData, FragmentShaderKey};

/// The palette shader, computing texture coordinates from an input color.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShaderData {
    pub u_min: [f32; 4],
    pub u_max: [f32; 4],
    pub v_min: [f32; 4],
    pub v_max: [f32; 4],
    pub uv_min: [f32; 4],
    pub uv_max: [f32; 4],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shader {
    pub data: ShaderData,
    pub spirv: Arc<[u8]>,
}

impl ShaderData {
    pub fn mgrid(width: f32, height: f32) -> Self {
        ShaderData {
            u_min: [0.0; 4],
            u_max: [width, 0.0, 0.0, 0.0],
            v_min: [0.0; 4],
            v_max: [0.0, height, 0.0, 0.0],
            uv_min: [0.0; 4],
            uv_max: [0.0; 4],
        }
    }

    pub fn into_std430(&self) -> Vec<u8> {
        let mat = [
            self.u_min,
            self.u_max,
            self.v_min,
            self.v_max,
            self.uv_min,
            self.uv_max,
        ];

        bytemuck::bytes_of(&mat).to_vec()
    }
}

impl FragmentShaderData for Shader {
    fn key(&self) -> Option<FragmentShaderKey> {
        Some(FragmentShaderKey::Bilinear)
    }

    fn spirv_source(&self) -> Arc<[u8]> {
        self.spirv.clone()
    }

    fn binary_data(&self, buffer: &mut Vec<u8>) -> Option<BufferInitContent> {
        let mat = self.data.into_std430();
        Some(BufferInitContent::new(buffer, &mat))
    }

    fn num_args(&self) -> u32 {
        0
    }
}
