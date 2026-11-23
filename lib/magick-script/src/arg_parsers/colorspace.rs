use std::{ffi::OsStr, fmt::Display, str::FromStr};

use crate::arg_parse_err::ArgParseErr;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Colorspace {
    pub cicp: Cicp,
    /// Corresponds to a ColorType in the image crate. Note that the depth is not actually encoded
    /// in the argument so you should adapt this with the setting `-depth`.
    pub color: ColorModel,
}

// FIXME: use a crate for this, again. Previously this linked against the whole `image` crate which
// is undesirable. We may move things to `image-core`, including the Cicp model, or this might use a
// more appropriate 4-byte Cicp struct from `image-canvas`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cicp {
    Srgb,
    SrgbLinear,
    DisplayP3,
}

impl Display for Colorspace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?} ({:?})", self.color, self.cicp)
    }
}

impl TryFrom<&OsStr> for Colorspace {
    type Error = ArgParseErr;

    fn try_from(value: &OsStr) -> Result<Self, Self::Error> {
        value
            .to_str()
            .ok_or_else(ArgParseErr::new)
            .and_then(FromStr::from_str)
    }
}

impl FromStr for Colorspace {
    type Err = ArgParseErr;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // <https://imagemagick.org/script/command-line-options.php#colorspace>
        match s.to_lowercase().as_str() {
            "srgb" => Ok(Colorspace {
                cicp: Cicp::Srgb,
                color: ColorModel::Rgb,
            }),
            "displayp3" => Ok(Colorspace {
                cicp: Cicp::DisplayP3,
                color: ColorModel::Rgb,
            }),
            "rgb" => Ok(Colorspace {
                cicp: Cicp::SrgbLinear,
                color: ColorModel::Luma,
            }),
            "gray" => Ok(Colorspace {
                cicp: Cicp::Srgb,
                color: ColorModel::Luma,
            }),
            "lineargray" => Ok(Colorspace {
                cicp: Cicp::SrgbLinear,
                color: ColorModel::Luma,
            }),
            _ => Err(ArgParseErr::with_msg(format!(
                "unsupported colorspace `{}`",
                s
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorModel {
    Luma,
    Rgb,
}

// Combination of specified `-storage-type`, `-depth`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChannelFormat {
    U8,
    #[expect(dead_code, reason = "-depth option introduced in future commits")]
    U16,
}
