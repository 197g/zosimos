use zosimos::command::Linker;
use zosimos::shaders::{ShadersCore, ShadersStd, stage::Shaders as ShadersStage};

pub fn from_included() -> &'static Linker {
    static INSTANCE: std::sync::OnceLock<Linker> = std::sync::OnceLock::new();
    INSTANCE.get_or_init(|| Linker {
        core: included_shaders_core(),
        std: included_shaders_std(),
    })
}

/// A vertex box shader, rendering a sole quad with given vertex and uv coordinate system.
pub const VERT_NOOP: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/spirv/box.vert.v"));

/// A 'noop' copy from the sampled texture to the output color based on the supplied UVs.
pub const FRAG_COPY: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/spirv/copy.frag.v"));
#[allow(dead_code)]
pub const FRAG_MIX_RGBA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/spirv/inject.frag.v"));
/// a linear transformation on rgb color.
pub const FRAG_LINEAR: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/spirv/linear.frag.v"));

mod shader {
    pub const BILINEAR: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/spirv/bilinear.frag.v"));

    pub const BOX: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/spirv/box3.frag.v"));

    pub const DISTRIBUTION_NORMAL_2D: &[u8] = include_bytes!(concat!(
        env!("OUT_DIR"),
        "/spirv/distribution_normal2d.frag.v"
    ));

    pub const FRACTAL_NOISE: &[u8] =
        include_bytes!(concat!(env!("OUT_DIR"), "/spirv/fractal_noise.frag.v"));

    pub const INJECT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/spirv/inject.frag.v"));

    pub const OKLAB_ENCODE: &[u8] =
        include_bytes!(concat!(env!("OUT_DIR"), "/spirv/oklab_encode.frag.v"));
    pub const OKLAB_DECODE: &[u8] =
        include_bytes!(concat!(env!("OUT_DIR"), "/spirv/oklab_decode.frag.v"));

    pub const PALETTE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/spirv/palette.frag.v"));

    pub const SOLID_RGBA: &[u8] =
        include_bytes!(concat!(env!("OUT_DIR"), "/spirv/solid_rgb.frag.v"));

    pub const SRLAB2_ENCODE: &[u8] =
        include_bytes!(concat!(env!("OUT_DIR"), "/spirv/srlab2_encode.frag.v"));
    pub const SRLAB2_DECODE: &[u8] =
        include_bytes!(concat!(env!("OUT_DIR"), "/spirv/srlab2_decode.frag.v"));
}

pub fn stage() -> ShadersStage {
    ShadersStage {
        decode_r8ui_x4: std::sync::Arc::from(
            &include_bytes!(concat!(env!("OUT_DIR"), "/spirv/stage_d8ui.frag.v"))[..],
        ),
        decode_r16ui_x2: std::sync::Arc::from(
            &include_bytes!(concat!(env!("OUT_DIR"), "/spirv/stage_d16ui.frag.v"))[..],
        ),
        decode_r32ui: std::sync::Arc::from(
            &include_bytes!(concat!(env!("OUT_DIR"), "/spirv/stage_d32ui.frag.v"))[..],
        ),
        encode_r8ui_x4: std::sync::Arc::from(
            &include_bytes!(concat!(env!("OUT_DIR"), "/spirv/stage_e8ui.frag.v"))[..],
        ),
        encode_r16ui_x2: std::sync::Arc::from(
            &include_bytes!(concat!(env!("OUT_DIR"), "/spirv/stage_e16ui.frag.v"))[..],
        ),
        encode_r32ui: std::sync::Arc::from(
            &include_bytes!(concat!(env!("OUT_DIR"), "/spirv/stage_e32ui.frag.v"))[..],
        ),
    }
}

pub fn included_shaders_core() -> ShadersCore {
    ShadersCore {
        vert_noop: VERT_NOOP.into(),
        frag_copy: FRAG_COPY.into(),
        frag_mix_rgba: FRAG_MIX_RGBA.into(),
        frag_linear: FRAG_LINEAR.into(),
        stage: stage(),
    }
}

pub fn included_shaders_std() -> ShadersStd {
    ShadersStd {
        bilinear: shader::BILINEAR.into(),
        box3: shader::BOX.into(),
        distribution_normal2d: shader::DISTRIBUTION_NORMAL_2D.into(),
        fractal_noise: shader::FRACTAL_NOISE.into(),
        inject: shader::INJECT.into(),
        linear_color_transform: FRAG_LINEAR.into(),
        oklab_encode: shader::OKLAB_ENCODE.into(),
        oklab_decode: shader::OKLAB_DECODE.into(),
        palette: shader::PALETTE.into(),
        solid_rgb: shader::SOLID_RGBA.into(),
        srlab2_encode: shader::SRLAB2_ENCODE.into(),
        srlab2_decode: shader::SRLAB2_DECODE.into(),
    }
}
