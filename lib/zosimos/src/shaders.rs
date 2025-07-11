use crate::color_matrix::RowMatrix;
use crate::program::BufferInitContent;
use std::sync::Arc;

pub mod bilinear;
pub mod box3;
pub mod distribution_normal2d;
pub mod fractal_noise;
pub mod inject;
pub mod oklab;
pub mod palette;
pub mod solid_rgb;
pub mod srlab2;
pub mod stage;

/// All the programs we need for the core language, i.e. everything that is not functions but just
/// managing the buffers, moving between bytes and textures type system.
#[derive(Clone)]
pub struct ShadersCore {
    pub vert_noop: Arc<[u8]>,
    pub frag_copy: Arc<[u8]>,
    pub frag_mix_rgba: Arc<[u8]>,
    pub frag_linear: Arc<[u8]>,
    pub stage: stage::Shaders,
}

#[derive(Clone)]
pub struct ShadersStd {
    pub bilinear: Arc<[u8]>,
    pub box3: Arc<[u8]>,
    pub distribution_normal2d: Arc<[u8]>,
    pub fractal_noise: Arc<[u8]>,
    pub inject: Arc<[u8]>,
    pub linear_color_transform: Arc<[u8]>,
    pub oklab_encode: Arc<[u8]>,
    pub oklab_decode: Arc<[u8]>,
    pub palette: Arc<[u8]>,
    pub solid_rgb: Arc<[u8]>,
    pub srlab2_encode: Arc<[u8]>,
    pub srlab2_decode: Arc<[u8]>,
}

/// A simple shader invocation.
///
/// This represents _one_ instance of a shader invocation. The compilation will evaluate the
/// methods to determine how the invocation is executed by the runtime pipeline.
///
/// FIXME: deriving PartialEq may be inferior to an actual implementation.
#[derive(Clone, Debug, PartialEq)]
pub struct ShaderInvocation {
    /// The shader source, shared between all instances of this similar invocation.
    pub(crate) spirv: Arc<[u8]>,
    /// The specific data of this invocation.
    pub(crate) shader_data: Option<Box<[u8]>>,
    /// The number of arguments (i.e. bound image samplers) that this shader is going to require.
    /// The encoder will match it to the actual number of input arguments later.
    pub(crate) num_args: u32,
}

/// A simplification of a fragment shader interface.
pub(crate) trait FragmentShaderData: core::fmt::Debug {
    /// The unique key identifying this shader pipeline setup.
    /// If two invocations return the same key then they are optimized and _not_ recompiled.
    /// Instead, we reuse setup from a previous shader module creation.
    fn key(&self) -> Option<FragmentShaderKey>;

    /// The SPIR-V shader source code.
    fn spirv_source(&self) -> Arc<[u8]>;

    /// Encode the shader's data into the buffer, returning the descriptor to that.
    ///
    /// FIXME: context of the buffer is imported. It may be possible to re-use a previous
    /// allocation iff the data is read-only and within the same encoding buffer.
    fn binary_data(&self, _: &mut Vec<u8>) -> Option<BufferInitContent> {
        None
    }

    /// Number of argument images consumed by the shader.
    /// This must match the number of arguments provided as `High::PushOperand`.
    fn num_args(&self) -> u32 {
        1
    }
}

impl FragmentShaderData for ShaderInvocation {
    fn key(&self) -> Option<FragmentShaderKey> {
        Some(FragmentShaderKey::Dynamic(self.spirv.as_ptr() as usize))
    }

    fn spirv_source(&self) -> Arc<[u8]> {
        self.spirv.clone()
    }

    fn binary_data(&self, buffer: &mut Vec<u8>) -> Option<BufferInitContent> {
        if let Some(boxed) = &self.shader_data {
            Some(BufferInitContent::new(buffer, boxed))
        } else {
            None
        }
    }

    fn num_args(&self) -> u32 {
        self.num_args
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub(crate) enum Direction {
    Encode = 0,
    Decode = 1,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum FragmentShaderKey {
    PaintOnTop(PaintOnTopKind),
    /// Linear color transformation.
    LinearColorMatrix,
    /// The conversion of texel format.
    /// FIXME: there are multiple sources of this.
    Convert(Direction, stage::StageKind),
    /// The generic distribution normal 2d.
    DistributionNormal2d,
    /// Generic fractal brownian noise.
    FractalNoise,
    /// Sample discrete colors from a palette.
    Palette,
    /// A bilinear function of colors.
    Bilinear,
    /// A shader mixing two colors, logically injecting ones channel into the other.
    Inject,
    /// A shader transforming between XYZ and Oklab color space.
    OklabTransform(Direction),
    /// A shader transforming between XYZ and SrLab2 color space.
    Srlab2Transform(Direction),
    /// A convolution with a 3-by-3 box function.
    Box3,
    /// The key is the address of some dynamic object, unique for the duration of the pipeline.
    /// One shouldn't rely on uniqueness of soundness.
    Dynamic(usize),
}

/// Identifies the vertex shading.
///
/// Currently, we only paint a single quad and all coordinates are encoded in the vertex buffer.
/// However, other directions we may proceed with:
/// - A transformation matrix so that the quad buffer is reusable.
/// - Non-Quad vertex shading.
/// - Different UV variants that require vertex shading, such as with depths?
/// - Instanced rendering where multiple quads are painted at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VertexShader {
    Noop,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum FragmentShaderInvocation {
    PaintOnTop(PaintOnTopKind),
    LinearColorMatrix(LinearColorTransform),
    Normal2d(DistributionNormal2d),
    FractalNoise(FractalNoise),
    Palette(self::palette::Shader),
    Bilinear(self::bilinear::Shader),
    Inject(self::inject::Shader),
    Oklab(self::oklab::Shader),
    SrLab2(self::srlab2::Shader),
    Box3(self::box3::Shader),
    SolidRgb(self::solid_rgb::Shader),
    Runtime(ShaderInvocation),
}

impl FragmentShaderInvocation {
    pub(crate) fn shader(&self) -> &dyn FragmentShaderData {
        match self {
            FragmentShaderInvocation::PaintOnTop(kind) => kind,
            FragmentShaderInvocation::LinearColorMatrix(shader) => shader,
            FragmentShaderInvocation::Normal2d(normal) => normal,
            FragmentShaderInvocation::FractalNoise(noise) => noise,
            FragmentShaderInvocation::Palette(palette) => palette,
            FragmentShaderInvocation::Bilinear(bilinear) => bilinear,
            FragmentShaderInvocation::Inject(inject) => inject,
            FragmentShaderInvocation::Oklab(oklab) => oklab,
            FragmentShaderInvocation::SrLab2(srlab2) => srlab2,
            FragmentShaderInvocation::Box3(box3) => box3,
            FragmentShaderInvocation::SolidRgb(color) => color,
            FragmentShaderInvocation::Runtime(dynamic) => dynamic,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PaintOnTopKind {
    Copy { spirv: Arc<[u8]> },
}

impl ShadersCore {
    pub(crate) fn paint_copy(&self) -> PaintOnTopKind {
        PaintOnTopKind::Copy {
            spirv: self.frag_copy.clone(),
        }
    }
}

impl FragmentShaderData for PaintOnTopKind {
    fn key(&self) -> Option<FragmentShaderKey> {
        Some(FragmentShaderKey::PaintOnTop(self.clone()))
    }

    fn spirv_source(&self) -> Arc<[u8]> {
        match self {
            PaintOnTopKind::Copy { spirv } => spirv.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LinearColorTransform {
    pub matrix: RowMatrix,
    pub spirv: Arc<[u8]>,
}

impl FragmentShaderData for LinearColorTransform {
    fn key(&self) -> Option<FragmentShaderKey> {
        Some(FragmentShaderKey::LinearColorMatrix)
    }

    fn spirv_source(&self) -> Arc<[u8]> {
        self.spirv.clone()
    }

    fn binary_data(&self, buffer: &mut Vec<u8>) -> Option<BufferInitContent> {
        let rgb_matrix: [f32; 12] = self.matrix.into_mat3x3_std140();
        Some(BufferInitContent::new(buffer, &rgb_matrix))
    }
}

pub(crate) use self::distribution_normal2d::Shader as DistributionNormal2d;
pub(crate) use self::fractal_noise::Shader as FractalNoise;
