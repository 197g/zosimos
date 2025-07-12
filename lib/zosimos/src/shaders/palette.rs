use std::sync::Arc;

use super::{BufferInitContent, FragmentShaderData, FragmentShaderKey};
use crate::buffer::{ChannelPosition, ColorChannel};

/// The palette shader, computing texture coordinates from an input color.
#[derive(Clone, Debug, PartialEq)]
pub struct ShaderData {
    pub(crate) x_coord: [f32; 4],
    pub(crate) y_coord: [f32; 4],
    pub(crate) base_x: i32,
    pub(crate) base_y: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Shader {
    pub data: ShaderData,
    pub spirv: Arc<[u8]>,
}

impl ChannelPosition {
    /// Find the channel index in the normalized, linear representation of said color.
    ///
    /// The caller in `command` is responsible for ensuring
    ///
    /// Reminder: we are looking for the position of the color channel in the _linear_
    /// representation of the color, i.e. within the vec4 loaded from the sampled texture.
    // For further colors later.
    #[allow(unreachable_patterns)]
    pub(crate) fn new(channel: ColorChannel) -> Option<Self> {
        use ColorChannel as Cc;
        Some(match channel {
            Cc::R => ChannelPosition::First,
            Cc::G => ChannelPosition::Second,
            Cc::B => ChannelPosition::Third,
            _ => return None,
        })
    }

    pub(crate) fn into_vec4(self) -> [f32; 4] {
        let mut p = [0.0; 4];
        p[self as usize] = 1.0;
        p
    }
}

impl FragmentShaderData for Shader {
    fn key(&self) -> Option<FragmentShaderKey> {
        Some(FragmentShaderKey::Palette)
    }

    fn spirv_source(&self) -> Arc<[u8]> {
        self.spirv.clone()
    }

    #[rustfmt::skip]
    fn binary_data(&self, buffer: &mut Vec<u8>) -> Option<BufferInitContent> {
        let mat4x2 = [
            self.data.x_coord[0], self.data.y_coord[0],
            self.data.x_coord[1], self.data.y_coord[1],
            self.data.x_coord[2], self.data.y_coord[2],
            self.data.x_coord[3], self.data.y_coord[3],
        ];

        Some(BufferInitContent::new(buffer, &mat4x2))
    }

    fn num_args(&self) -> u32 {
        2
    }
}
