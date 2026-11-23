//! A proxy module of what should probably be `image-core` crate.
use std::ffi::OsStr;
use std::ffi::OsString;

pub(crate) use image_canvas::color::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
}

impl ImageFormat {
    pub(crate) fn from_extension(ext: &OsStr) -> Option<Self> {
        match ext.to_str().map(|s| s.to_lowercase()).as_deref() {
            Some("png") => Some(Self::Png),
            Some("jpg" | "jpeg") => Some(Self::Jpeg),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct InputProperties {
    pub filename: OsString,
    pub color_type: image_canvas::color::Color,
}

#[derive(Debug, Clone)]
pub struct Image {
    // TODO: ImageFormat only lists the built-in formats, which is why it's an Option.
    // We need the extended format enum with a string for plug-in formats here, but it's not public (yet).
    pub format: Option<ImageFormat>,
    pub exif: Option<Vec<u8>>,
    pub icc: Option<Vec<u8>>,
    pub pixels: DynamicImage,
    pub properties: InputProperties,
}

#[derive(Clone)]
pub struct DynamicImage(image_canvas::Canvas);

impl core::fmt::Debug for DynamicImage {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DynamicImage")
            .field("width", &self.0.layout().width())
            .field("height", &self.0.layout().height())
            .field("color_type", &self.0.layout().color())
            .finish()
    }
}
