use crate::{
    arg_parsers::{
        BlurGeometry, CropGeometry, Filter, GrayscaleMethod, IdentifyFormat, LoadCropGeometry,
        ResizeGeometry, UnsharpenGeometry,
    },
    image::Color,
    plan,
};

#[derive(Debug, Clone, PartialEq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Operation {
    Resize(ResizeGeometry, Option<Filter>),
    Thumbnail(ResizeGeometry, Option<Filter>),
    Scale(ResizeGeometry),
    Sample(ResizeGeometry),
    CropOnLoad(LoadCropGeometry),
    Crop(CropGeometry),
    Identify(Option<IdentifyFormat>),
    Negate,
    AutoOrient,
    Blur(BlurGeometry),
    GaussianBlur(BlurGeometry),
    Grayscale(GrayscaleMethod),
    Flip(Axis),
    Monochrome,
    Unsharpen(UnsharpenGeometry),
}

impl Operation {
    /// Modifiers are flags such as -quality that affect operations.
    /// For global operations we need to alter them after the operation's creation,
    /// to apply up-to-date modifiers.
    pub fn apply_modifiers(&mut self, mods: &plan::Modifiers) {
        use Operation::*;
        match self {
            Resize(resize_geometry, _) => *self = Resize(*resize_geometry, mods.filter),
            Thumbnail(resize_geometry, _) => *self = Thumbnail(*resize_geometry, mods.filter),
            Scale(_) => (),
            Sample(_) => (),
            CropOnLoad(_) => (),
            Crop(_) => (),
            Identify(_) => *self = Identify(mods.identify_format.clone()),
            Negate => (),
            AutoOrient => (),
            Blur(_) => (),
            GaussianBlur(_) => (),
            Grayscale(_) => (),
            Flip(_) => (),
            Monochrome => (),
            Unsharpen(_) => (),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RewriteOperation {
    Combine {
        color: Color,
        /// Rewrite the color model to true color (`sRGB`) when the channel count is exceeded?
        fallback_for_channel_count: bool,
    },
}
