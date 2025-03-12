mod dynamic;

pub use self::dynamic::{ShaderCommand, ShaderData, ShaderSource};

use crate::buffer::{ByteLayout, CanvasLayout, ChannelPosition, Descriptor, TexelExt};
use crate::color_matrix::RowMatrix;
use crate::pool::PoolImage;
use crate::program::{
    BufferWrite, ByteBufferAssignment, CallBinding, CompileError, Frame, Function, FunctionLinked,
    High, ImageBufferAssignment, ImageBufferPlan, ImageDescriptor, Initializer, Knob, KnobUser,
    ParameterizedFragment, Program, QuadTarget, RegisterAssignment, Target, Texture,
};
pub use crate::shaders::bilinear::Shader as Bilinear;
pub use crate::shaders::distribution_normal2d::Shader as DistributionNormal2d;
pub use crate::shaders::fractal_noise::Shader as FractalNoise;

use crate::shaders::{self, FragmentShaderInvocation, PaintOnTopKind, ShaderInvocation};

use image_canvas::color::{Color, ColorChannel, Whitepoint};
use image_canvas::layout::{SampleParts, Texel};

use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;

/// A reference to one particular value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Register(pub(crate) usize);

/// One linear sequence of instructions.
///
/// The machine model is a single basic block in SSA where registers are strongly typed with their
/// buffer descriptors.
///
/// *Why not a … stack machine*: The author believes that stack machines are a poor fit for image
/// editing in general. Their typical core assumption is that a) registers have the same size b)
/// copying them is cheap. Neither is true for images.
///
/// *Why not a … mutable model*: Because this would complicate the tracking of types. Due to the
/// lack of loops there is little reason for mutability anyways. If you wrap the program in a loop
/// to simulate branches yourself then each `launch` provides the opportunity to rebind the images
/// or bind an image to an output, where it can be mutated.
///
/// The strict typing and SSA-liveness analysis allows for a clean analysis of required temporary
/// resources, re-usage of those as an internal optimization, and in general simple analysis and
/// verification.
#[derive(Default)]
pub struct CommandBuffer {
    ops: Vec<Op>,
    vars: Vec<TyVarBounds>,
    symbols: Vec<CommandSignature>,
    tys: Vec<GenericDescriptor>,
    /// Commands that consume a statically initialized buffer, which we can adjust at launch time.
    knobs: HashMap<Register, KnobKind>,
}

/// Refers to a generic argument declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GenericVar(pub(crate) usize);

/// Refers to the descriptor introduced by a generic argument or a derived var.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DescriptorVar(pub(crate) usize);

/// Refers to the function introduced by its signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FunctionVar(pub(crate) usize);

pub struct CommandSignature {
    vars: Vec<TyVarBounds>,
    input: Vec<GenericDescriptor>,
    output: Vec<GenericDescriptor>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GenericDescriptor {
    size: Generic<(u32, u32)>,
    chroma: Generic<(Texel, Color)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GenericBuffer {
    /// The size of this buffer, where statically known.
    ///
    /// Generic means the size is determined by some other sized type parameter. Descriptor
    /// parameters are sized in terms of their IO encoded buffer. Buffer parameters are sized in
    /// terms of their own size.
    size: Generic<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Generic<T> {
    Concrete(T),
    Generic(GenericVar),
}

/// Describe the value class of a register, and its precise type.
#[non_exhaustive]
pub enum RegisterDescription<'cmd> {
    /// This register is not a value operation.
    None,
    /// This register is a texture.
    Texture(&'cmd GenericDescriptor),
    /// This register is a byte-based buffer.
    Buffer(&'cmd GenericBuffer),
}

#[derive(Clone, Debug)]
enum Op {
    /// i := in()
    Input { desc: GenericDescriptor },
    /// out(src)
    ///
    /// WIP: and is_cpu_type(desc)
    /// for the eventuality of gpu-only buffer layouts.
    Output { src: Register },
    /// target(src)
    /// where is_linear_type(src)
    ///
    /// for the eventuality of gpu-only buffer layouts.
    /// FIXME: already contain proof of is_linear_type?
    Render { src: Register },
    /// i := op()
    /// where type(i) = desc
    Construct {
        desc: GenericDescriptor,
        op: ConstructOp,
    },
    /// i := unary(src)
    /// where type(i) =? Op[type(src)]
    Unary {
        src: Register,
        op: UnaryOp,
        desc: GenericDescriptor,
    },
    /// i := binary(lhs, rhs)
    /// where type(i) =? Op[type(lhs), type(rhs)]
    Binary {
        lhs: Register,
        rhs: Register,
        op: BinaryOp,
        desc: GenericDescriptor,
    },
    DynamicImage {
        call: OperandDynKind,
        /// The planned shader invocation.
        command: ShaderInvocation,
        desc: GenericDescriptor,
    },
    Invoke {
        function: FunctionVar,
        arguments: Vec<Register>,
        results: Vec<Register>,
        generics: Vec<GenericDescriptor>,
    },
    /// The specific return value of a function.
    InvokedResult {
        /// Where is this register initialized? Must be after its definition.
        invocation: Register,
        /// The result's type monomorphized as called.
        desc: GenericDescriptor,
    },
    BufferInit {
        op: BufferInitOp,
        desc: GenericBuffer,
    },
    BufferUnary {
        src: Register,
        op: BufferUnaryOp,
        desc: GenericBuffer,
    },
    BufferBinary {
        lhs: Register,
        rhs: Register,
        op: BufferBinaryOp,
        desc: GenericBuffer,
    },
}

enum KnobKind {
    Runtime,
    Buffer {
        buffer: Register,
        range: core::ops::Range<u64>,
    },
}

#[derive(Clone)]
struct TyVarBounds {}

/// Declare a fresh generic declaration parameter.
pub struct GenericDeclaration<'lt> {
    pub bounds: &'lt [GenericBound],
}

/// Declare a new descriptor type based on a generic bound.
pub struct DescriptorDerivation {
    pub base: DescriptorVar,
    pub size: Option<(u32, u32)>,
}

pub enum GenericBound {}

#[derive(Clone, Debug)]
enum OperandDynKind {
    Construct,
    Unary(Register),
    Binary { lhs: Register, rhs: Register },
}

pub struct InvocationArguments<'lt> {
    pub generics: &'lt [DescriptorVar],
    pub arguments: &'lt [Register],
}

#[derive(Clone, Debug)]
pub(crate) enum ConstructOp {
    Bilinear(Bilinear),
    /// A 2d normal distribution.
    DistributionNormal(shaders::DistributionNormal2d),
    /// Fractal noise
    DistributionNoise(shaders::FractalNoise),
    /// A color to repeat on pixels.
    Solid([f32; 4]),
    /// An existing buffer to use.
    FromBuffer(Register),
}

#[derive(Clone, Debug)]
pub(crate) enum BufferInitOp {
    FromData {
        placement: core::ops::Range<usize>,
        data: Arc<[u8]>,
    },
}

#[derive(Clone, Debug)]
pub(crate) enum BufferUnaryOp {
    FromImage {},
}

#[derive(Clone, Debug)]
pub(crate) enum BufferBinaryOp {
    /// Combine two buffers by overlaying one over the contents of the other, at a fixed location.
    Overlay { at: u64 },
}

#[derive(Clone, Debug)]
pub(crate) enum UnaryOp {
    /// Op = id
    Crop(Rectangle),
    /// Op(color)[T] = T[.color=color]
    /// And color needs to be 'color compatible' with the prior T (see module).
    ColorConvert(ColorConversion),
    /// Op(T) = T[.color=select(channel, color)]
    #[allow(dead_code)] // "See discussion in its usage. The selection happens in the shader."
    Extract { channel: ChannelPosition },
    /// Op(T) = T[.whitepoint=target]
    /// This is a partial method for CIE XYZ-ish color spaces. Note that ICC requires adaptation to
    /// D50 for example as the reference color space.
    ChromaticAdaptation(ChromaticAdaptation),
    /// Op(T) = T
    Vignette(VignetteRemoval),
    /// Op(T) = T[.texel=texel]
    /// And the byte width of new texel must be consistent with the current byte width.
    Transmute,
    /// Op(T) = T
    Derivative(Derivative),
}

#[derive(Clone, Debug)]
pub(crate) enum BinaryOp {
    /// Op = id
    Affine(Affine),
    /// Op[T, U] = T
    /// where T = U
    Inscribe { placement: Rectangle },
    /// Replace a channel T with U itself.
    /// Op[T, U] = T
    /// where select(channel, T.color) = U.color
    Inject {
        channel: ChannelPosition,
        from_channels: Texel,
    },
    /// Sample from a palette based on the color value of another image.
    /// Op[T, U] = T
    Palette(shaders::PaletteShader),
    /// Apply gain map.
    ///
    /// Op[T, U] = T
    GainMap(GainMap),
}

/// A rectangle in `u32` space.
/// It's describe by minimum and maximum coordinates, inclusive and exclusive respectively. Any
/// rectangle where the order is not correct is interpreted as empty. This has the advantage of
/// simplifying certain operations that would otherwise need to check for correctness.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rectangle {
    pub x: u32,
    pub y: u32,
    pub max_x: u32,
    pub max_y: u32,
}

#[derive(Clone, Copy)]
#[non_exhaustive]
pub enum Blend {
    Alpha,
}

/// Describes an affine transformation of an image.
///
/// Affine transformations are a combination of scaling, translation, rotation. They describe a
/// transformation of the 2D space of the original image.
#[derive(Clone, Copy, Debug)]
pub struct Affine {
    /// The affine transformation, as a row-major homogeneous matrix.
    ///
    /// Note that the top-left pixel starts at (0, 0), the bottom right pixel ends at (1, 1).
    pub transformation: [f32; 9],
    /// How pixels are resolved from the underlying texture.
    pub sampling: AffineSample,
}

/// The way to perform sampling of an texture that was transformed with an affine transformation.
///
/// You have to be careful that there is NO built-in functionality to avoid attacks that downscale
/// an image so far that a very particular subset of pixels (or linear interpolation) is shown that
/// results in an image visually very different from the original. Such an attack works because
/// scaling down leads to many pixels being ignored.
#[derive(Clone, Copy, Debug)]
pub enum AffineSample {
    /// Choose the nearest pixel.
    ///
    /// This method works with all color models.
    Nearest,
    /// Interpolate bi-linearly between nearest pixels.
    ///
    /// We rely on the executing GPU sampler2D for determining the color, in particular it will happen
    /// in _linear_ RGB and this method can only be used on RGB-ish images.
    BiLinear,
}

/// The parameters of color conversion which we will use in the draw call.
#[derive(Clone, Debug)]
pub(crate) enum ColorConversion {
    Xyz {
        /// The matrix converting source to XYZ.
        to_xyz_matrix: RowMatrix,
        /// The matrix converting from XYZ to target.
        from_xyz_matrix: RowMatrix,
    },
    XyzToOklab {
        /// The matrix converting source to XYZ.
        to_xyz_matrix: RowMatrix,
    },
    OklabToXyz {
        /// The matrix converting from XYZ to target.
        from_xyz_matrix: RowMatrix,
    },
    XyzToSrLab2 {
        /// The matrix converting source to XYZ.
        to_xyz_matrix: RowMatrix,
        /// The SrLAb2 target whitepoint.
        whitepoint: Whitepoint,
    },
    SrLab2ToXyz {
        /// The matrix converting from XYZ to target.
        from_xyz_matrix: RowMatrix,
        /// The SrLAb2 source whitepoint.
        whitepoint: Whitepoint,
    },
}

/// Reference of matrices and more: http://brucelindbloom.com/index.html?Eqn_ChromAdapt.html
///
/// A similar technique can simulate cone deficiencies:
/// * deuteranomaly (green cone cells defective),
/// * protanomaly (red cone cells defective),
/// * and tritanomaly (blue cone cells defective).
/// More information here: http://colorspace.r-forge.r-project.org/articles/color_vision_deficiency.html
///
/// Matrix for transforming cone response into the opponent color space which is assumed to be a
/// mostly sufficient input to recreate a particular color impression. In other words, simulate a
/// dichromacy response to a color by matching it for a 'standard' human. We can also estimate if a
/// particular color can be made visible for someone afflicted with a cone deficiency etc.
/// (see: Gustavo M. Machado, et.al A Physiologically-based Model for Simulation of Color Vision Deficiency)
///
/// ```text
/// 0.600    0.400    0.000
/// 0.240    0.105   −0.700
/// 1.200   −1.600    0.400
/// ```
#[derive(Clone, Debug)]
pub(crate) struct ChromaticAdaptation {
    /// The matrix converting source to XYZ.
    to_xyz_matrix: RowMatrix,
    /// The target whitepoint of the adaptation.
    source: Whitepoint,
    /// The method to use.
    method: ChromaticAdaptationMethod,
    /// The matrix converting from XYZ to target.
    from_xyz_matrix: RowMatrix,
    /// The target whitepoint of the adaptation.
    target: Whitepoint,
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum ChromaticAdaptationMethod {
    /// Naive adaptation based on component-wise linear transform in XYZ.
    Xyz,
    /// A component-wise transform in LMS (cone response) coordinates.
    ///
    /// The matrix for whitepoint E (equal intensity) is:
    ///
    /// ```latex
    /// \begin{bmatrix}
    /// 0.38971 & 0.68898 & -0.07868\\
    /// -0.22981 & 1.18340 & 0.04641\\
    /// 0.00000 & 0.00000 & 1.00000
    /// \end{bmatrix}
    /// ```
    ///
    /// The D65 normalized XYZ -> LMS matrix is:
    ///
    /// ```text
    /// 0.4002400  0.7076000 -0.0808100
    /// -0.2263000  1.1653200  0.0457000
    /// 0.0000000  0.0000000  0.9182200
    /// ```
    VonKries,
    /// Bradford's modified (sharpened) LMS definition with linear VonKries adaptation.
    /// Used in ICC.
    ///
    /// ```latex
    /// \begin{bmatrix}
    /// 0.8951 & 0.2664 & -0.1614 \\
    /// -0.7502 & 1.7135 & 0.0367 \\
    /// 0.0389 & -0.0685 & 1.0296
    /// \end{bmatrix}
    /// ```
    BradfordVonKries,
    /// Bradford's originally intended adaptation.
    BradfordNonLinear,
}

/// Remove spherical differences in effective irradiation.
///
/// There are a handful of common models for the difference in apparent pixel brightness caused by
/// this effect. These are probably not accurate to the actual optics and build of your camera.
/// Realistically one should calibrate the method to known images or between a target sequence—and
/// in particular choose a good enough model in this step.
///
/// Note that vignetting is caused by the physical make of the camera and lense system, e.g.
/// partial shadowing of light paths into a pixel by the shutter and lense body. That is it will
/// *very* across different focal lengths and lenses.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum VignetteRemoval {
    /// The irradiance is by four components with the constant held at `1`.
    Polynom3 { coefficients: [f32; 3] },
}

/// Defines a gain map metadata to apply.
///
/// A gain map is a pixel-weighted rescaling factor encoded in logarithmic scale. Applying such a
/// map in the forward direction entails using the SDR pixel data, applying a bias, multiplying,
/// and applying a new bias. The gain factor may be sampled by some interpolation method in the
/// logarithmic space of the gain map. Each _color_ channel may be scaled individually.
///
/// For a specification: <https://helpx.adobe.com/camera-raw/using/gain-map.html>
///
/// ```text
/// # defined by the integer-encoded matrix:
/// G(x) = x / (2**N – 1)
/// # load by our interpolation method:
/// y = sample(img_coord, gain_map)
/// # apply some transform from the metadata
/// z = affine_gain_transform(y, metadata)
/// # determine an output-specified weight based on the intent of the metadata
/// w = intent(output, metadata)
/// # transform the sdr into hdr
/// 𝐻𝐷𝑅 = (𝑆𝐷𝑅 + 𝑘𝑠𝑑𝑟) ⋅ 2**𝐺(y*w) − 𝑘ℎ𝑑𝑟
/// ```
///
/// Note that some producers may fix the bias values `ksdr` and `khdr` to small values below the
/// quantization range of the image during the encoding. This avoids mathematical poles in
/// determining the gain which calculates `log2(sdr - khdr)` and `log2(hdr - khdr)`.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct GainMap {
    /// The sdr bias as defined in metadata or broadcast across channels.
    bias_sdr: [f32; 3],
    /// The hdr bias as defined in metadata or broadcast across channels.
    bias_hdr: [f32; 3],
    /// The HDR range as intended by the metadata.
    intent_hdr_range: core::ops::Range<f32>,
    /// The HDR capacity of our target output image, that is log2 ration of its HDR to SDR gain.
    display_hdr_capacity: core::ops::Range<f32>,
    /// The gain logarithmic affine parameters, lower bound.
    gain_min: f32,
    /// The gain logarithmic affine parameters, upper bound.
    gain_max: f32,
    /// The gain logarithmic affine parameters, exponent scaling.
    gain_gamma: f32,
}

/// A palette lookup operation.
///
/// FIXME description and implementation
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// Which color channel will provide the texture coordinate along width axis.
    pub width: Option<ColorChannel>,
    /// Which color channel will provide the texture coordinate along height axis.
    pub height: Option<ColorChannel>,
    /// The base coordinate for sampling along width.
    pub width_base: i32,
    /// The base coordinate for sampling along height.
    pub height_base: i32,
    // FIXME: wrapping?
}

/// Calculate a first derivative.
#[derive(Clone, Debug, Hash)]
pub struct Derivative {
    pub method: DerivativeMethod,
    pub direction: Direction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Along the height of the image.
    Height,
    /// Along the width of the image.
    Width,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DerivativeMethod {
    /// A 2-sized filter with diagonal basis direction.
    ///     [[1 0] [0 -1]]
    ///     [[0 1] [-1 0]]
    Roberts,
    /// The result of derivative with average smoothing.
    ///     1/6 [1 1 1]^T [1 0 -1]
    Prewitt,
    /// The result of derivative with weighted smoothing.
    ///     1/8 [1 2 1]^T [1 0 -1]
    Sobel,
    /// The Scharr 3×3 operator, floating point precision with the error bound of the paper.
    ///
    /// There is a single numerically consistent choice in differentiation but the smoothing for
    /// rotational symmetry is:
    ///     [46.84 162.32 46.84]/256
    ///
    /// It is optimized to most accurately model the perfect transfer function `pi·i·w` (the first
    /// derivative operator) while providing the ability to steer the direction. That is, you can
    /// calculate the derivative into any direction by composing Dx and Dy. The weights for this
    /// optimization had been `w(k) = ∏i:(1 to D) cos^4(π/2·k_i)` where `D = 2` in the flat image
    /// case.
    ///
    /// It's a bit constricting to use it here because on the GPU integer arithmetic is _NOT_ the
    /// most efficient form of computation (which is the motivation behind using a quantized
    /// integer matrix). They nevertheless exist for compatibility reasons.
    ///
    /// Reference: Scharr's dissertation, Optimale Operatoren in der Digitalen Bildverarbeitung
    /// <https://doi.org/10.11588/heidok.00000962>
    /// <http://archiv.ub.uni-heidelberg.de/volltextserver/962/1/Diss.pdf>
    Scharr3,
    /// A 4-tab derivative operator by Scharr.
    /// * Derivative: `[77.68 139.48 …]/256`
    /// * Smoothing: `[16.44 111.56 …]/256`
    Scharr4,
    /// A 5-tab derivative by Scharr.
    /// * Derivative: `[21.27 85.46 0 …]/256`
    /// * Smoothing: `[5.91 61.77 120.64 …]/256`
    Scharr5,
    /// The 4-bit approximated Scharr operator.
    ///     1/32 [3 10 3]^T [1 0 -1]
    ///
    /// This is provided for compatibility! For accuracy you may instead prefer to use Scharr3 or
    /// Schar5Tab.
    Scharr3To4Bit,
    /// A non-smoothed 5-tab derivative.
    ///     `[-0.262 1.525 0 -1.525 0.262]`
    Scharr5Tab,
    /// The 8-bit approximated Scharr operator.
    ///
    /// This is provided for compatibility! For accuracy you may instead prefer to use Scharr3.
    Scharr3To8Bit,
}

/// Methods for removing noise from an image.
///
/// WIP: these are not yet implemented.
///
/// This intuitive understanding applies to single valued, gray scale images. The operator will
/// also work for any colored images as long as the color space defines a luminance, lightness,
/// or value channel. We will then choose a pixel by median of that channel.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SmoothingMethod {
    /// Also called: average, arithmetic mean.
    Laplace,
    /// Weighted average using a gauss kernel.
    Gaussian,
    /// Choose the median value from surrounding pixels.
    ///
    /// The choice is made through the Luma channel.
    Median,
    /// Bilateral filter, weighting pixels by values.
    ///
    /// The weighting is made through the Luma channel.
    Bilteral,
    /// Chooses a value from the surrounding region with minimal variance.
    ///
    /// This implements Kuwahara's initial approach where the representation is chosen to be the
    /// mean value and the regions are defined as exactly 4 regions overlapping on the axes and the
    /// pixel itself.
    ///
    /// Using hexadecimal bit masks to represent the regions, an example with 3×3 regions:
    ///
    /// ```text
    /// 11322
    /// 11322
    /// 55faa
    /// 44c88
    /// 44c88
    /// ```
    Kuwahara,
}

#[derive(Debug)]
pub struct CommandError {
    inner: CommandErrorKind,
}

/// Generic instantiation that is todo by the linker.
struct CommandMonomorphization<'lt> {
    /// The name of the buffer in the linker.
    link_idx: usize,
    command: &'lt CommandBuffer,
    tys: Cow<'lt, [Descriptor]>,
}

#[derive(Hash, PartialEq, Eq)]
struct LinkedMonomorphicSignature {
    /// The function, by its Linker symbol index.
    link_idx: usize,
    tys: Vec<Descriptor>,
}

struct Monomorphizing<'lt> {
    stack: Vec<CommandMonomorphization<'lt>>,
    monomorphic: HashMap<LinkedMonomorphicSignature, Function>,
    commands: Vec<&'lt CommandBuffer>,
    knobs: HashMap<RegisterKnob, Knob>,
    next_knob: Knob,
    /// The ID of a program being translated.
    current_link_id: usize,
}

/// Schedule instructions with host-dependent and modifiable configuration.
pub struct WithKnob<'lt> {
    inner: &'lt mut CommandBuffer,
}

/// Schedule instructions with parameters already in GPU memory.
#[allow(dead_code)]
pub struct WithBuffer<'lt> {
    inner: &'lt mut CommandBuffer,
    guaranteed_len: u64,
    start: u64,
    /// The register supplying the buffer value.
    register: Register,
}

/// A register that may be mapped to a knob.
///
/// A `knob` is the identifier of a region of memory _in the linked program_, associated with some
/// set of static parameters in that source. You can discover the identifiers chosen during linking
/// by passing the index and register of the [`CommandBuffer`] which has created its command with a
/// corresponding [`WithKnob`] method. Not all commands can have knobs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RegisterKnob {
    pub link_idx: usize,
    pub register: Register,
}

#[derive(Debug)]
// `Debug` is our use. Until we get better errors.
#[allow(unused)]
enum CommandErrorKind {
    BadDescriptor(GenericDescriptor, &'static str),
    ConcreteDescriptorRequired,
    ConflictingTypes(GenericDescriptor, GenericDescriptor),
    GenericTypeError,
    Other,
    Unimplemented,
}

/// Intrinsically defined methods of manipulating images.
///
/// FIXME: missing functions
/// - a way to represent colors as a function of wavelength, then evaluate with the standard
/// observers.
/// - implement simulation of color blindness (reuses matrix multiplication, related to observer)
/// - generate color ramps
/// - color interpolation along a cubic spline (how do we represent the parameters?)
/// - hue shift, transforms on the a*b* circle, such as mobius transform (z-a)/(1-z·adj(a)) i.e or
///   other holomorphic functions. Ways to construct mobius transfrom from three key points. That
///   is particular relevant for color correction.
///
/// For developers aiming to add extensions to the system, see the other impl-block.
///
/// The order of arguments is generally
/// 1. Input argument
///   a. Configuration of first argument.
///   b. Configuration of second argument …
///   c. … (no operator with more argument atm)
/// 2. Arguments to the command itself
impl CommandBuffer {
    /// Declare an input.
    ///
    /// Inputs MUST later be bound from the pool during launch.
    pub fn input(&mut self, desc: Descriptor) -> Result<Register, CommandError> {
        if !desc.is_consistent() {
            return Err(CommandError {
                inner: CommandErrorKind::BadDescriptor(desc.into(), "inconsistent input declared"),
            });
        }

        Ok(self.push(Op::Input { desc: desc.into() }))
    }

    /// Declare an input that depends on a generic descriptor parameter.
    ///
    /// See [`Self::generic`].
    pub fn input_generic(&mut self, var: DescriptorVar) -> Result<Register, CommandError> {
        let Some(desc) = self.tys.get(var.0).cloned() else {
            return Err(CommandError::BAD_REGISTER);
        };

        Ok(self.push(Op::Input { desc }))
    }

    /// Declare a generic parameter.
    ///
    /// All generic parameters need to be filled with matching concrete variables when the function
    /// is instantiated at a later point.
    pub fn generic(&mut self, generic: GenericDeclaration) -> DescriptorVar {
        for item in generic.bounds {
            match *item {}
        }

        let bounds = TyVarBounds {};
        let tyvar = GenericVar(self.vars.len());
        self.vars.push(bounds);

        let descriptor = DescriptorVar(self.tys.len());
        self.tys.push(GenericDescriptor {
            size: Generic::Generic(tyvar),
            chroma: Generic::Generic(tyvar),
        });

        descriptor
    }

    /// Create a descriptor var by modifying another.
    pub fn derive_descriptor(
        &mut self,
        var: DescriptorDerivation,
    ) -> Result<DescriptorVar, CommandError> {
        let DescriptorDerivation { base, size } = var;
        let Some(from) = self.tys.get(base.0).cloned() else {
            return Err(CommandError::BAD_REGISTER);
        };

        let desc = GenericDescriptor {
            size: size.map_or(from.size, Generic::Concrete),
            chroma: from.chroma,
        };

        let descriptor = DescriptorVar(self.tys.len());
        self.tys.push(desc);

        Ok(descriptor)
    }

    /// Create a descriptor var, describing a previous register.
    pub fn register_descriptor(
        &mut self,
        register: Register,
    ) -> Result<DescriptorVar, CommandError> {
        let generic = self.describe_reg(register).as_texture()?;

        let descriptor = DescriptorVar(self.tys.len());
        self.tys.push(generic.clone());

        Ok(descriptor)
    }

    /// Calculate the signature based on generics, inputs, outputs.
    pub fn computed_signature(&self) -> CommandSignature {
        CommandSignature {
            vars: self.vars.clone(),
            input: self
                .ops
                .iter()
                .filter_map(|op| {
                    if let Op::Input { desc } = op {
                        Some(desc.clone())
                    } else {
                        None
                    }
                })
                .collect(),
            output: self
                .ops
                .iter()
                .filter_map(|op| {
                    if let &Op::Output { src } = op {
                        Some(
                            // FIXME: non-texture output.
                            self.describe_reg(src)
                                .as_texture()
                                .expect("Validated when creating output")
                                .clone(),
                        )
                    } else {
                        None
                    }
                })
                .collect(),
        }
    }

    /// Declare a function that is later linked in.
    pub fn function(&mut self, signature: CommandSignature) -> Result<FunctionVar, CommandError> {
        let symbol = FunctionVar(self.symbols.len());
        self.symbols.push(signature);
        Ok(symbol)
    }

    /// Declare an invocation of a separate, possibly generic, function.
    pub fn invoke(
        &mut self,
        function: FunctionVar,
        invoke: InvocationArguments,
    ) -> Result<Vec<Register>, CommandError> {
        let signature = self
            .symbols
            .get(function.0)
            .ok_or(CommandError::BAD_REGISTER)?;

        if signature.input.len() != invoke.arguments.len() {
            return Err(CommandError::INVALID_CALL);
        }

        if signature.vars.len() != invoke.generics.len() {
            return Err(CommandError::INVALID_CALL);
        }

        let generics: Vec<_> = invoke
            .generics
            .iter()
            .map(|&DescriptorVar(idx)| self.tys.get(idx).cloned())
            .collect::<Option<_>>()
            .ok_or(CommandError::BAD_REGISTER)?;

        for (tyvar, tyarg) in signature.vars.iter().zip(invoke.generics) {
            // FIXME: fully validate generics. We need to know specifics about a type variable, in
            // particular its locally known bounds, and a system to query these bounds. We just
            // incidentally do not yet have any such bounds.
            let _ = (tyvar, tyarg);
        }

        for (param, arg) in signature.input.iter().zip(invoke.arguments) {
            let expected = param.rewrite(&generics);
            let arg_ty = self.describe_reg(*arg).as_texture()?;

            if expected != *arg_ty {
                return Err(CommandError::INVALID_CALL);
            }
        }

        let invocation = Register(self.ops.len() + signature.output.len());

        let mut results = vec![];
        for (_result_idx, output) in signature.output.iter().enumerate() {
            results.push(Register(self.ops.len()));
            let desc = output.rewrite(&generics);

            self.ops.push(Op::InvokedResult { invocation, desc });
        }

        self.ops.push(Op::Invoke {
            function,
            arguments: invoke.arguments.to_vec(),
            results: results.clone(),
            generics,
        });

        Ok(results)
    }

    /// Declare an image as input.
    ///
    /// Returns its register if the image has a valid descriptor, otherwise panics.
    pub fn input_from(&mut self, img: PoolImage) -> Register {
        let descriptor = img.descriptor();
        self.input(descriptor)
            .expect("Pool image descriptor should be valid")
    }

    /// Construct an image buffer a buffer already allocated in device memory.
    ///
    /// Note that the buffer must describe the image as it is allocated in the (encoded) transfer
    /// buffer of the device memory. That is, each row is padded and aligned.
    pub fn from_buffer(
        &mut self,
        buffer_reg: Register,
        descriptor: Descriptor,
    ) -> Result<Register, CommandError> {
        let RegisterDescription::Buffer(buffer) = self.describe_reg(buffer_reg) else {
            return Err(CommandError::TYPE_ERR);
        };

        let gpu_layout = descriptor
            .to_aligned()
            .ok_or_else(|| CommandError::INVALID_CALL)?;

        let required_size = u64::from(gpu_layout.height)
            .checked_mul(gpu_layout.row_stride)
            .ok_or_else(|| CommandError::TYPE_ERR)?;

        match buffer.size {
            Generic::Concrete(sz) if sz >= required_size => {}
            Generic::Concrete(_) => {
                return Err(CommandError::INVALID_CALL);
            }
            _ => {
                return Err(CommandError::UNIMPLEMENTED);
            }
        }

        Ok(self.push(Op::Construct {
            desc: descriptor.into(),
            op: ConstructOp::FromBuffer(buffer_reg),
        }))
    }

    /// Select a rectangular part of an image.
    pub fn crop(&mut self, src: Register, rect: Rectangle) -> Result<Register, CommandError> {
        let desc = self.describe_reg(src).as_texture()?.clone();
        Ok(self.push(Op::Unary {
            src,
            op: UnaryOp::Crop(rect),
            desc,
        }))
    }

    /// Create an image with different color encoding.
    ///
    /// This goes through linear RGB, not ICC, and requires the two models to have same whitepoint.
    ///
    /// Note that this is not a generic operation. It selects the conversion based on the input
    /// type which requires it to have a concrete descriptor.
    pub fn color_convert(
        &mut self,
        src: Register,
        color: Color,
        texel: Texel,
    ) -> Result<Register, CommandError> {
        let desc_src = self.describe_reg(src).as_texture()?;
        let conversion;

        let desc_src = desc_src.as_concrete().ok_or(CommandError {
            inner: CommandErrorKind::ConcreteDescriptorRequired,
        })?;

        // Pretend that all colors with the same whitepoint will be mapped from encoded to
        // linear RGB when loading, and re-encoded in target format when storing them. This is
        // almost correct, but not all GPUs will support all texel kinds. In particular
        // some channel orders or bit-field channels are likely to be unsupported. In these
        // cases, we will later add some temporary conversion.
        //
        // FIXME: this is growing a bit ugly with non-rgb color spaces. We should find a more
        // general way to handle these, and in particular also handle non-rgb-to-non-rgb because
        // that's already happening in the command encoder anyways..

        match (&desc_src.color, &color) {
            (
                Color::Rgb {
                    primary: primary_src,
                    whitepoint: wp_src,
                    ..
                },
                Color::Rgb {
                    primary: primary_dst,
                    whitepoint: wp_dst,
                    ..
                },
            ) if wp_src == wp_dst => {
                conversion = ColorConversion::Xyz {
                    from_xyz_matrix: RowMatrix(primary_src.to_xyz_row_matrix(*wp_src)),
                    to_xyz_matrix: RowMatrix(primary_dst.to_xyz_row_matrix(*wp_dst)),
                };
            }
            (
                Color::Rgb {
                    primary,
                    whitepoint: Whitepoint::D65,
                    ..
                },
                Color::Oklab,
            ) => {
                conversion = ColorConversion::XyzToOklab {
                    to_xyz_matrix: RowMatrix(primary.to_xyz_row_matrix(Whitepoint::D65)),
                };
            }
            (
                Color::Oklab,
                Color::Rgb {
                    primary,
                    whitepoint: Whitepoint::D65,
                    ..
                },
            ) => {
                conversion = ColorConversion::OklabToXyz {
                    from_xyz_matrix: RowMatrix(primary.to_xyz_row_matrix(Whitepoint::D65)),
                };
            }
            (
                Color::Rgb {
                    primary,
                    whitepoint: rgb_wp,
                    ..
                },
                Color::SrLab2 { whitepoint },
            ) => {
                conversion = ColorConversion::XyzToSrLab2 {
                    to_xyz_matrix: RowMatrix(primary.to_xyz_row_matrix(*rgb_wp)),
                    whitepoint: *whitepoint,
                };
            }
            (
                Color::SrLab2 { whitepoint },
                Color::Rgb {
                    primary,
                    whitepoint: rgb_wp,
                    ..
                },
            ) => {
                conversion = ColorConversion::SrLab2ToXyz {
                    from_xyz_matrix: RowMatrix(primary.to_xyz_row_matrix(*rgb_wp)),
                    whitepoint: *whitepoint,
                };
            }
            _ => {
                return Err(CommandError {
                    inner: CommandErrorKind::BadDescriptor(
                        desc_src.clone().into(),
                        "No conversion",
                    ),
                })
            }
        }

        // FIXME: validate memory condition.
        let layout = ByteLayout {
            width: desc_src.layout.width,
            height: desc_src.layout.height,
            texel_stride: texel.bits.bytes(),
            row_stride: desc_src.layout.width as u64 * texel.bits.bytes() as u64,
        };

        let op = Op::Unary {
            src,
            op: UnaryOp::ColorConvert(conversion),
            desc: Descriptor {
                color,
                layout,
                texel,
            }
            .into(),
        };

        Ok(self.push(op))
    }

    /// Perform a whitepoint adaptation.
    ///
    /// The `function` describes the method and target whitepoint of the chromatic adaptation.
    pub fn chromatic_adaptation(
        &mut self,
        src: Register,
        method: ChromaticAdaptationMethod,
        target: Whitepoint,
    ) -> Result<Register, CommandError> {
        let desc_src = self.describe_reg(src).as_texture()?;
        let texel_color;
        let source_wp;
        let (to_xyz_matrix, from_xyz_matrix);

        let desc_src = desc_src.as_concrete().ok_or(CommandError {
            inner: CommandErrorKind::ConcreteDescriptorRequired,
        })?;

        match desc_src.color {
            Color::Rgb {
                whitepoint,
                primary,
                transfer,
                luminance,
            } => {
                texel_color = Color::Rgb {
                    whitepoint: target,
                    primary,
                    transfer,
                    luminance,
                };

                to_xyz_matrix = RowMatrix(primary.to_xyz_row_matrix(whitepoint));
                from_xyz_matrix = RowMatrix(primary.from_xyz_row_matrix(target));
                source_wp = whitepoint;
            }
            // Forward compatibility.
            _ => {
                return Err(CommandError {
                    inner: CommandErrorKind::BadDescriptor(
                        desc_src.clone().into(),
                        "non-rgb chromatic adaptation",
                    ),
                })
            }
        };

        let desc = Descriptor {
            color: texel_color,
            ..desc_src.clone()
        };

        let op = Op::Unary {
            src,
            op: UnaryOp::ChromaticAdaptation(ChromaticAdaptation {
                to_xyz_matrix,
                source: source_wp,
                target,
                from_xyz_matrix,
                method,
            }),
            desc: desc.into(),
        };

        Ok(self.push(op))
    }

    /// Embed this image as part of a larger one.
    pub fn inscribe(
        &mut self,
        below: Register,
        rect: Rectangle,
        above: Register,
    ) -> Result<Register, CommandError> {
        let desc_below = self.describe_reg(below).as_texture()?;
        let desc_above = self.describe_reg(above).as_texture()?;

        if desc_above.descriptor_chroma() != desc_below.descriptor_chroma() {
            return Err(CommandError {
                inner: CommandErrorKind::ConflictingTypes(desc_below.clone(), desc_above.clone()),
            });
        }

        let desc_above = desc_above.as_concrete().ok_or(CommandError {
            inner: CommandErrorKind::ConcreteDescriptorRequired,
        })?;

        if Rectangle::with_layout(&desc_above.layout) != rect {
            return Err(CommandError::OTHER);
        }

        // This is pretty much lint status, actually. Nothing intensely bad happens if we paint
        // outside the image, we could just paint less of it.
        if let Some(concrete) = desc_below.as_concrete() {
            if !Rectangle::with_layout(&concrete.layout).contains(rect) {
                return Err(CommandError::OTHER);
            }
        }

        let op = Op::Binary {
            lhs: below,
            rhs: above,
            op: BinaryOp::Inscribe {
                placement: rect.normalize(),
            },
            desc: desc_below.clone(),
        };

        Ok(self.push(op))
    }

    /// Extract some channels from an image data into a new view.
    pub fn extract(
        &mut self,
        src: Register,
        channel: ColorChannel,
    ) -> Result<Register, CommandError> {
        let desc_src = self.describe_reg(src).as_texture()?;

        let desc_src = desc_src.as_concrete().ok_or(CommandError {
            inner: CommandErrorKind::ConcreteDescriptorRequired,
        })?;

        let texel = desc_src
            .texel
            .channel_texel(channel)
            .ok_or(CommandError::OTHER)?;

        let layout = ByteLayout {
            texel_stride: texel.bits.bytes(),
            width: desc_src.layout.width,
            height: desc_src.layout.height,
            row_stride: (texel.bits.bytes() as u64) * u64::from(desc_src.layout.width),
        };

        let color = desc_src.color.clone();

        // Check that we can actually extract that channel.
        // This could be unimplemented if the position of a particular channel is not yet a stable
        // detail. Also, we might introduce 'virtual' channels such as `Luminance` on an RGB image
        // where such channels are computed by linear combination instead of a binary incidence
        // vector. Then there might be colors where this does not exist.
        let channel = ChannelPosition::new(channel).ok_or(CommandError::OTHER)?;

        let op = Op::Unary {
            src,
            op: UnaryOp::Extract { channel },
            desc: Descriptor {
                color,
                layout,
                texel,
            }
            .into(),
        };

        Ok(self.push(op))
    }

    /// Reinterpret the bytes of an image as another type.
    ///
    /// This command requires that the texel type of the register and the descriptor have the same
    /// size. It will return an error if this is not the case. Additionally, the provided texel
    /// must be internally consistent.
    ///
    /// One important use of this method is to add or removed the color interpretation of an image.
    /// This can be necessary when it has been algorithmically created or when one wants to
    /// intentionally ignore such meaning.
    pub fn transmute(
        &mut self,
        src: Register,
        target: Descriptor,
    ) -> Result<Register, CommandError> {
        self.transmute_generic(src, target.into())
    }

    /// Reinterpret the bytes of an image as another type.
    ///
    /// Like [`Self::transmute`] except the target can be a generic. Note however that it must be
    /// provable that the texels contain the same number of bytes and align in their storage layout
    /// (see [`SampleBits::bytes`]). This requires both texel types to be concrete or to be the
    /// exact same generic.
    ///
    /// Other methods for demonstrating this as a bound might be added at a later point but are
    /// essentially a form of dependent typing, so don't count too much on it.
    pub fn transmute_generic(
        &mut self,
        src: Register,
        into: GenericDescriptor,
    ) -> Result<Register, CommandError> {
        let source = self.describe_reg(src).as_texture()?;
        let supposed_type = into;

        if source.size() != supposed_type.size() {
            return Err(CommandError {
                inner: CommandErrorKind::BadDescriptor(
                    supposed_type,
                    "invalid transmute with mismatched size",
                ),
            });
        }

        // Predict if monomorphize will only do correct transmutes. A transmute re-interprets the
        // buffer containing bit data in storage layout.
        fn can_transmute(source: Generic<(Texel, Color)>, target: Generic<(Texel, Color)>) -> bool {
            match (source, target) {
                (Generic::Generic(vsource), Generic::Generic(vtarget)) => vsource == vtarget,
                (Generic::Concrete((source, _)), Generic::Concrete((target, _))) => {
                    source.bits.bytes() == target.bits.bytes()
                }
                _ => false,
            }
        }

        if !can_transmute(
            source.descriptor_chroma(),
            supposed_type.descriptor_chroma(),
        ) {
            return Err(CommandError {
                inner: CommandErrorKind::ConflictingTypes(source.clone(), supposed_type),
            });
        }

        if !supposed_type
            .as_concrete()
            .map_or(true, |descriptor| descriptor.is_consistent())
        {
            return Err(CommandError {
                inner: CommandErrorKind::BadDescriptor(
                    supposed_type,
                    "invalid transmute with inconsistent result",
                ),
            });
        }

        let op = Op::Unary {
            src,
            op: UnaryOp::Transmute,
            desc: supposed_type,
        };

        Ok(self.push(op))
    }

    /// Overwrite some channels with overlaid data.
    ///
    /// This performs an implicit conversion of the overlaid data to the color channels which is
    /// performed as if by transmutation. However, contrary to the transmutation we will _only_
    /// allow the sample parts to be changed arbitrarily.
    ///
    /// To perform a mix of two images with differing texels or colors, as if by rendering rather
    /// than as if by transmute, use `mix` [FIXME: not yet implemented].
    pub fn inject(
        &mut self,
        below: Register,
        channel: ColorChannel,
        above: Register,
    ) -> Result<Register, CommandError> {
        let desc_below = self.describe_reg(below).as_texture()?;
        let desc_above = self.describe_reg(above).as_texture()?.clone();

        let Generic::Concrete((below_texel, below_color)) = desc_below.descriptor_chroma() else {
            return Err(CommandError {
                inner: CommandErrorKind::BadDescriptor(
                    desc_below.clone(),
                    "inject into non-concrete texel",
                ),
            });
        };

        let Generic::Concrete((above_texel, above_color)) = desc_above.descriptor_chroma() else {
            return Err(CommandError {
                inner: CommandErrorKind::BadDescriptor(
                    desc_above.clone(),
                    "inject from non-concrete texel",
                ),
            });
        };

        let expected_texel = below_texel
            .channel_texel(channel)
            .ok_or(CommandError::OTHER)?;

        if above_texel.parts.num_components() != expected_texel.parts.num_components() {
            let wanted = GenericDescriptor {
                chroma: Generic::Concrete((expected_texel, below_color)),
                ..desc_below.clone()
            };

            return Err(CommandError {
                inner: CommandErrorKind::ConflictingTypes(wanted, desc_above),
            });
        }

        let from_channels = above_texel.clone();
        // Override the sample part interpretation for comparison. We ignore this and compare
        // everything else. This is because we change specifically the parts by this operation.
        let mut above_texel = above_texel;
        above_texel.parts = expected_texel.parts;

        // FIXME: should we do parsing instead of validation?
        // Some type like ChannelPosition but for multiple.
        if from_channels.channel_weight_vec4().is_none() {
            return Err(CommandError::OTHER);
        }

        if (&expected_texel, &below_color) != (&above_texel, &above_color) {
            let wanted = GenericDescriptor {
                chroma: Generic::Concrete((expected_texel, below_color)),
                ..desc_below.clone()
            };

            return Err(CommandError {
                inner: CommandErrorKind::ConflictingTypes(wanted, desc_above),
            });
        }

        // Find where to insert, see `extract` for this step.
        let channel = ChannelPosition::new(channel).ok_or(CommandError::OTHER)?;

        let op = Op::Binary {
            lhs: below,
            rhs: above,
            op: BinaryOp::Inject {
                channel,
                from_channels,
            },
            desc: desc_below.clone(),
        };

        Ok(self.push(op))
    }

    /// Grab colors from a palette based on an underlying image of indices.
    pub fn palette(
        &mut self,
        palette: Register,
        config: Palette,
        indices: Register,
    ) -> Result<Register, CommandError> {
        let color_desc = self.describe_reg(palette).as_texture()?;
        let idx_desc = self.describe_reg(indices).as_texture()?;

        // FIXME: check that channels are actually in indices' color type.
        let x_coord = if let Some(coord) = config.width {
            let pos = ChannelPosition::new(coord).ok_or(CommandError::TYPE_ERR)?;
            pos.into_vec4()
        } else {
            [0.0; 4]
        };

        let y_coord = if let Some(coord) = config.height {
            let pos = ChannelPosition::new(coord).ok_or(CommandError::TYPE_ERR)?;
            pos.into_vec4()
        } else {
            [0.0; 4]
        };

        // Compute the target layout (and that we can represent it).
        let target_layout = GenericDescriptor {
            chroma: color_desc.descriptor_chroma(),
            ..idx_desc.clone()
        };

        let op = Op::Binary {
            lhs: palette,
            rhs: indices,
            op: BinaryOp::Palette(shaders::PaletteShader {
                x_coord,
                y_coord,
                base_x: config.width_base,
                base_y: config.height_base,
            }),
            desc: target_layout,
        };

        Ok(self.push(op))
    }

    /// Calculate the derivative of an image.
    ///
    /// Currently, will only calculate the derivative for color channels. The alpha channel will be
    /// copied from the source pixel. To also calculate a derivative over the alpha channel you
    /// should extract it as a value channel, calculate the derivative there and the inject the
    /// result back to the image.
    pub fn derivative(
        &mut self,
        image: Register,
        config: Derivative,
    ) -> Result<Register, CommandError> {
        let desc = self.describe_reg(image).as_texture()?.clone();

        let op = Op::Unary {
            src: image,
            op: UnaryOp::Derivative(config),
            desc,
        };

        Ok(self.push(op))
    }

    /// Overlay this image as part of a larger one, performing blending.
    pub fn blend(
        &mut self,
        _below: Register,
        _rect: Rectangle,
        _above: Register,
        _blend: Blend,
    ) -> Result<Register, CommandError> {
        // TODO: What blending should we support
        Err(CommandError::UNIMPLEMENTED)
    }

    /// A solid color image, from a descriptor and a single color.
    ///
    /// Repeats the color across all pixels, then transforms into equivalent texels.
    pub fn solid_rgba(
        &mut self,
        describe: Descriptor,
        color: [f32; 4],
    ) -> Result<Register, CommandError> {
        if !describe.is_consistent() {
            return Err(CommandError {
                inner: CommandErrorKind::BadDescriptor(
                    describe.into(),
                    "inconsistent constant color image created",
                ),
            });
        }

        if color.len() != usize::from(describe.layout.texel_stride) {
            return Err(CommandError {
                inner: CommandErrorKind::BadDescriptor(
                    describe.into(),
                    "inconsistent color description",
                ),
            });
        }

        Ok(self.push(Op::Construct {
            desc: describe.into(),
            op: ConstructOp::Solid(color.to_owned()),
        }))
    }

    /// A 2d image with a normal distribution.
    ///
    /// The parameters are controlled through the `distribution` parameter while the `texel`
    /// parameter controls the eventual binary encoding of the image. It must be compatible with a
    /// single gray channel (but you can have electrical transfer functions, choose arbitrary bit
    /// widths etc.).
    pub fn distribution_normal2d(
        &mut self,
        describe: Descriptor,
        distribution: shaders::DistributionNormal2d,
    ) -> Result<Register, CommandError> {
        if !describe.is_consistent() {
            return Err(CommandError {
                inner: CommandErrorKind::BadDescriptor(describe.into(), "inconsistent normal2d"),
            });
        }

        if describe.texel.parts != SampleParts::Luma && describe.texel.parts != SampleParts::LumaA {
            return Err(CommandError {
                inner: CommandErrorKind::BadDescriptor(
                    describe.into(),
                    "normal2d for non-LumA texel",
                ),
            });
        }

        Ok(self.push(Op::Construct {
            desc: describe.into(),
            op: ConstructOp::DistributionNormal(distribution),
        }))
    }

    /// A 2d image with fractal brownian noise.
    ///
    /// The parameters are controlled through the `distribution` parameter. Output contains
    /// in each of the 4 color channels uncorrelated, 1 dimensional fractal perlin noise.
    pub fn distribution_fractal_noise(
        &mut self,
        describe: Descriptor,
        distribution: shaders::FractalNoise,
    ) -> Result<Register, CommandError> {
        if !describe.is_consistent() {
            return Err(CommandError {
                inner: CommandErrorKind::BadDescriptor(
                    describe.into(),
                    "inconsistent descriptor for fractal noise",
                ),
            });
        }

        Ok(self.push(Op::Construct {
            desc: describe.into(),
            op: ConstructOp::DistributionNoise(distribution),
        }))
    }

    /// Evaluate a bilinear function over a 2d image.
    ///
    /// For each color channel, the parameter contains intervals of values that define how its
    /// value is determined along the width and height axis.
    ///
    /// This can be used similar to `numpy`'s `mgrid`.
    pub fn bilinear(
        &mut self,
        describe: Descriptor,
        distribution: Bilinear,
    ) -> Result<Register, CommandError> {
        if !describe.is_consistent() {
            return Err(CommandError {
                inner: CommandErrorKind::BadDescriptor(
                    describe.into(),
                    "inconsistent descriptor for bilinear",
                ),
            });
        }

        Ok(self.push(Op::Construct {
            desc: describe.into(),
            op: ConstructOp::Bilinear(distribution),
        }))
    }

    /// Overlay an affine transformation of the image.
    pub fn affine(
        &mut self,
        below: Register,
        affine: Affine,
        above: Register,
    ) -> Result<Register, CommandError> {
        // TODO: should we check affine here?
        let lhs = self.describe_reg(below).as_texture()?.clone();
        let rhs = self.describe_reg(above).as_texture()?.clone();

        if lhs.descriptor_chroma() != rhs.descriptor_chroma() {
            return Err(CommandError::TYPE_ERR);
        }

        match RowMatrix::new(affine.transformation)
            .det()
            .abs()
            .partial_cmp(&f32::EPSILON)
        {
            Some(Ordering::Greater | Ordering::Equal) => {}
            _ => return Err(CommandError::OTHER),
        }

        match affine.sampling {
            AffineSample::Nearest => (),
            AffineSample::BiLinear => {
                // "Check for a color which we can sample bi-linearly"
                return Err(CommandError::UNIMPLEMENTED);
            }
        }

        Ok(self.push(Op::Binary {
            lhs: below,
            rhs: above,
            op: BinaryOp::Affine(affine),
            desc: lhs,
        }))
    }

    pub fn resize(&mut self, below: Register, upper: (u32, u32)) -> Result<Register, CommandError> {
        let (width, height) = upper;
        let grid_layout = Descriptor::with_texel(Texel::new_u8(SampleParts::RgbA), width, height)
            .ok_or(CommandError::OTHER)?;

        let grid = self.bilinear(
            grid_layout,
            Bilinear {
                u_min: [0.0, 0.0, 0.0, 1.0],
                v_min: [0.0, 0.0, 0.0, 1.0],
                uv_min: [0.0, 0.0, 0.0, 1.0],
                u_max: [1.0, 0.0, 0.0, 1.0],
                v_max: [0.0, 1.0, 0.0, 1.0],
                uv_max: [0.0, 0.0, 0.0, 1.0],
            },
        )?;

        self.palette(
            below,
            Palette {
                width: Some(ColorChannel::R),
                height: Some(ColorChannel::G),
                width_base: 0,
                height_base: 0,
            },
            grid,
        )
    }

    /// Declare an output.
    ///
    /// Outputs MUST later be bound from the pool during launch.
    pub fn output(&mut self, src: Register) -> Result<(Register, GenericDescriptor), CommandError> {
        let outformat = self.describe_reg(src).as_texture()?.clone();
        // Ignore this, it doesn't really produce a register.
        let register = self.push(Op::Output { src });
        Ok((register, outformat))
    }

    /// Declare a render target.
    ///
    /// Render targets MUST later be bound from the pool during launch, similar to outputs. However, they are not assumed to be readable afterwards and will never be a copy target.
    ///
    /// The target register must be renderable, i.e. a color with a native texture representation.
    pub fn render(&mut self, src: Register) -> Result<(Register, Descriptor), CommandError> {
        let outformat = self.describe_reg(src).as_texture()?.clone();

        let outformat = outformat.as_concrete().ok_or(CommandError {
            inner: CommandErrorKind::ConcreteDescriptorRequired,
        })?;

        // FIXME: this is too conservative! We need to ensure that our internal assumption about
        // the texture descriptor is compatible with available wgpu formats (and yields the same
        // result).
        if ImageDescriptor::new(&outformat).is_err() {
            return Err(CommandError::TYPE_ERR);
        }

        // Ignore this, it doesn't really produce a register.
        let register = self.push(Op::Render { src });
        Ok((register, outformat))
    }

    /// Configure a next, parameterized, operation whose parameter structure can be overridden at
    /// runtime.
    pub fn with_knob(&mut self) -> WithKnob<'_> {
        WithKnob { inner: self }
    }

    /// Similar to `with_knob` but here we can use a different set of calls.
    ///
    /// The next parameterized operation is called with its parameter structure copied from the
    /// given buffer, instead of parameters supplied statically in the command buffer.
    ///
    /// Where it would be necessary to do indirect paint calls it'll get more complicated in the
    /// translation stage (need new `Low` ops) but it should be simple for a few other calls.
    pub fn with_buffer(&mut self, register: Register) -> Result<WithBuffer<'_>, CommandError> {
        let buffer = self.describe_reg(register).as_buffer()?;

        let len = buffer.as_concrete().ok_or(CommandError {
            inner: CommandErrorKind::ConcreteDescriptorRequired,
        })?;

        Ok(WithBuffer {
            inner: self,
            guaranteed_len: len,
            start: 0,
            register,
        })
    }
}

/// Commands that operate on buffers.
impl CommandBuffer {
    /// Construct a buffer by initializing it with data from memory.
    ///
    /// The binary value will be copied into a buffer held by the execution state. If you intend to
    /// modify that buffer with each execution, see [`Self::with_knob`] and [`WithKnob::buffer_init`].
    ///
    /// FIXME: late errors depending on `wgpu` since we copy the buffer and that requires it to be
    /// a multiple of `4`. This contradicts the notion that the hardware is chosen at a later
    /// stage.. We should instead compute?
    pub fn buffer_init(&mut self, init: &[u8]) -> Register {
        use core::convert::TryInto as _;
        let size: u64 = init.len().try_into().unwrap();

        self.push(Op::BufferInit {
            desc: GenericBuffer {
                size: Generic::Concrete(size),
            },
            op: BufferInitOp::FromData {
                placement: 0..init.len(),
                data: Arc::from(init),
            },
        })
    }

    /// Construct a buffer that is fully zeroed from memory.
    pub fn buffer_zero(&mut self, len: u64) -> Register {
        self.push(Op::BufferInit {
            desc: GenericBuffer {
                size: Generic::Concrete(len),
            },
            op: BufferInitOp::FromData {
                placement: 0..0,
                data: Arc::default(),
            },
        })
    }

    /// Construct a buffer representing *encoded* image data.
    ///
    /// FIXME: semantics of `Ok` depend on `wgpu`. This contradicts the notion that the hardware is
    /// chosen at a later stage..
    pub fn buffer_from_image(&mut self, register: Register) -> Result<Register, CommandError> {
        let RegisterDescription::Texture(tex) = self.describe_reg(register) else {
            return Err(CommandError::BAD_REGISTER);
        };

        let len = match tex.as_concrete() {
            Some(descriptor) => descriptor
                .u64_gpu_len()
                // Well can this even happen? A concrete image with no layout on the GPU?
                .ok_or_else(|| CommandError::INVALID_CALL)?,
            // FIXME: better diagnostic or allow this? We can't guarantee if this will error or not
            // and we can not give a concrete length for the buffer. Both must be decided in
            // some way
            None => return Err(CommandError::BAD_REGISTER),
        };

        Ok(self.push(Op::BufferUnary {
            src: register,
            desc: GenericBuffer {
                size: Generic::Concrete(len),
            },
            op: BufferUnaryOp::FromImage {},
        }))
    }

    /// Construct a buffer by overlaying one on top of another.
    ///
    /// The output buffer is sized according to the underlying buffer. Overflowed data will be
    /// discarded.
    pub fn buffer_overlay(
        &mut self,
        under: Register,
        at: u64,
        over: Register,
    ) -> Result<Register, CommandError> {
        let RegisterDescription::Buffer(buf) = self.describe_reg(under) else {
            return Err(CommandError::BAD_REGISTER);
        };

        let RegisterDescription::Buffer(_) = self.describe_reg(over) else {
            return Err(CommandError::BAD_REGISTER);
        };

        // FIXME: generate warnings if out of bounds? There is no use cloning a buffer that I can
        // see right now, it's all still the exact same content.
        Ok(self.push(Op::BufferBinary {
            lhs: under,
            rhs: over,
            desc: GenericBuffer {
                size: buf.size.clone(),
            },
            op: BufferBinaryOp::Overlay { at },
        }))
    }
}

impl WithKnob<'_> {
    /// Wrap commands that generate one register instruction, that is parameterized by the buffer.
    fn regular_with_knob(
        &mut self,
        fn_: impl FnOnce(&mut CommandBuffer) -> Result<Register, CommandError>,
    ) -> Result<Register, CommandError> {
        let register = fn_(&mut self.inner)?;
        self.inner.knobs.insert(register, KnobKind::Runtime);
        Ok(register)
    }

    /// See [`CommandBuffer::chromatic_adaptation`].
    ///
    /// FIXME: untested, does this make sense? Knob controls the color transformation matrix
    /// directly, not semantically.
    pub fn chromatic_adaptation(
        &mut self,
        src: Register,
        method: ChromaticAdaptationMethod,
        target: Whitepoint,
    ) -> Result<Register, CommandError> {
        self.regular_with_knob(move |cmd| cmd.chromatic_adaptation(src, method, target))
    }

    /// See [`CommandBuffer::inscribe`].
    ///
    /// FIXME: untested, does this make sense?
    pub fn inscribe(
        &mut self,
        below: Register,
        rect: Rectangle,
        above: Register,
    ) -> Result<Register, CommandError> {
        self.regular_with_knob(move |cmd| cmd.inscribe(below, rect, above))
    }

    /// See [`CommandBuffer::solid_rgba`].
    pub fn solid_rgba(
        &mut self,
        describe: Descriptor,
        color: [f32; 4],
    ) -> Result<Register, CommandError> {
        self.regular_with_knob(move |cmd| cmd.solid_rgba(describe, color))
    }

    /// See [`CommandBuffer::distribution_normal2d`].
    pub fn distribution_normal2d(
        &mut self,
        describe: Descriptor,
        distribution: shaders::DistributionNormal2d,
    ) -> Result<Register, CommandError> {
        self.regular_with_knob(move |cmd| cmd.distribution_normal2d(describe, distribution))
    }

    /// See [`CommandBuffer::distribution_fractal_noise`].
    pub fn distribution_fractal_noise(
        &mut self,
        describe: Descriptor,
        distribution: shaders::FractalNoise,
    ) -> Result<Register, CommandError> {
        self.regular_with_knob(move |cmd| cmd.distribution_fractal_noise(describe, distribution))
    }

    /// See [`CommandBuffer::bilinear`].
    pub fn bilinear(
        &mut self,
        describe: Descriptor,
        distribution: Bilinear,
    ) -> Result<Register, CommandError> {
        self.regular_with_knob(move |cmd| cmd.bilinear(describe, distribution))
    }

    /// See [`CommandBuffer::buffer_init`].
    pub fn buffer_init(&mut self, init: &[u8]) -> Result<Register, CommandError> {
        self.regular_with_knob(move |cmd| Ok(cmd.buffer_init(init)))
    }

    /*Should be knob'able but we currently do not generate the vertex coordinate buffer, i.e. sampled
     * 2d parameterization, in a manner that permits adding a knob.

        /// See [`CommandBuffer::crop`].
        pub fn crop(&mut self, src: Register, rect: Rectangle) -> Result<Register, CommandError> {
            self.regular_with_knob(move |cmd| cmd.crop(src, rect))
        }

        /// See [`CommandBuffer::affine`].
        pub fn affine(
            &mut self,
            below: Register,
            affine: Affine,
            above: Register,
        ) -> Result<Register, CommandError> {
            self.regular_with_knob(move |cmd| cmd.affine(below, affine, above))
        }

    */
}

impl WithBuffer<'_> {
    /// Wrap commands that generate one register instruction, that is parameterized by the buffer.
    fn regular_with_buffer(
        &mut self,
        len: u64,
        fn_: impl FnOnce(&mut CommandBuffer) -> Result<Register, CommandError>,
    ) -> Result<Register, CommandError> {
        if self.guaranteed_len < len {
            return Err(CommandError::INVALID_CALL);
        }

        let register = fn_(&mut self.inner)?;

        self.inner.knobs.insert(
            register,
            KnobKind::Buffer {
                buffer: self.register,
                range: 0..len,
            },
        );

        Ok(register)
    }

    /// Change the start of the buffer region being passed as dynamic value.
    pub fn with_start(self, start: u64) -> Result<Self, CommandError> {
        if start % 4 != 0 {
            return Err(CommandError::INVALID_CALL);
        }

        Ok(WithBuffer { start: 4, ..self })
    }

    /// See [`CommandBuffer::chromatic_adaptation`].
    pub fn chromatic_adaptation(
        &mut self,
        src: Register,
        method: ChromaticAdaptationMethod,
        target: Whitepoint,
    ) -> Result<Register, CommandError> {
        self.regular_with_buffer(core::mem::size_of::<[f32; 12]>() as u64, move |cmd| {
            cmd.chromatic_adaptation(src, method, target)
        })
    }

    /// See [`CommandBuffer::solid_rgba`].
    pub fn solid_rgba(
        &mut self,
        describe: Descriptor,
        color: [f32; 4],
    ) -> Result<Register, CommandError> {
        self.regular_with_buffer(core::mem::size_of::<[f32; 4]>() as u64, move |cmd| {
            cmd.solid_rgba(describe, color)
        })
    }

    /// See [`CommandBuffer::distribution_normal2d`].
    pub fn distribution_normal2d(
        &mut self,
        describe: Descriptor,
        distribution: shaders::DistributionNormal2d,
    ) -> Result<Register, CommandError> {
        self.regular_with_buffer(core::mem::size_of::<[f32; 8]>() as u64, move |cmd| {
            cmd.distribution_normal2d(describe, distribution)
        })
    }

    /// See [`CommandBuffer::distribution_fractal_noise`].
    pub fn distribution_fractal_noise(
        &mut self,
        describe: Descriptor,
        distribution: shaders::FractalNoise,
    ) -> Result<Register, CommandError> {
        #[repr(C)]
        #[repr(align(8))]
        struct _ForSizePurpose {
            _0: [f32; 2],
            _1: f32,
            _2: f32,
            _3: u32,
        }

        self.regular_with_buffer(core::mem::size_of::<_ForSizePurpose>() as u64, move |cmd| {
            cmd.distribution_fractal_noise(describe, distribution)
        })
    }

    /// See [`CommandBuffer::bilinear`].
    pub fn bilinear(
        &mut self,
        describe: Descriptor,
        distribution: Bilinear,
    ) -> Result<Register, CommandError> {
        self.regular_with_buffer(core::mem::size_of::<[[f32; 4]; 6]>() as u64, move |cmd| {
            cmd.bilinear(describe, distribution)
        })
    }
}

/// Turn a command buffer into a `Program`.
impl CommandBuffer {
    pub fn compile(&self) -> Result<Program, CompileError> {
        self.link(&[], &[], &[])
    }

    /// An unergonomic interface for linking a collection of different command buffers to a
    /// program. The `functions` are all buffers besides `self` that are linked. `links` describes
    /// the relation between them. For each buffer (`self` at 0 then incremented across the array)
    /// a list match all function declarations in that buffer to the command supplying the
    /// definition. The generic signature must match each declaration it is linked to.
    ///
    /// FIXME: higher level interface here. We should be able to configured links with pairs of a
    /// `FunctionVar` and a higher-level wrapper around a `CommandBuffer` index. Also it makes not
    /// much sense to treat the `self` special except as a defaulted entry point and for the
    /// `compile` helper that does not require any linkage.
    pub fn link(
        &self,
        tys: &[Descriptor],
        functions: &[CommandBuffer],
        links: &[&[usize]],
    ) -> Result<Program, CompileError> {
        // We can default to 'no links', which is fine..
        if functions.len() + 1 < links.len() {
            eprintln!("Bad link listings count");
            // Error: more links than functions..
            return Err(CompileError::NotYetImplemented);
        }

        let mut high_ops = vec![];

        let mut monomorphic = Monomorphizing {
            stack: vec![],
            monomorphic: HashMap::new(),
            commands: Some(self).into_iter().chain(functions).collect(),
            knobs: HashMap::new(),
            next_knob: Knob(0),
            current_link_id: 0,
        };

        monomorphic.push_function(LinkedMonomorphicSignature {
            link_idx: 0,
            tys: Cow::Borrowed(tys).into_owned(),
        });

        impl Monomorphizing<'_> {
            /// Assign a program function index to a specific generic instantiation.
            ///
            /// Remembers to process the monomorphization later if it was not instantiated yet.
            pub fn push_function(&mut self, sig: LinkedMonomorphicSignature) -> Function {
                let idx = self.monomorphic.len();

                let stack = &mut self.stack;
                let command = &self.commands[sig.link_idx];

                *self.monomorphic.entry(sig).or_insert_with_key(|key| {
                    stack.push(CommandMonomorphization {
                        link_idx: key.link_idx,
                        command,
                        tys: Cow::Owned(key.tys.to_vec()),
                    });

                    Function(idx)
                })
            }

            pub fn next_knob(&mut self, register: Register) -> Knob {
                let knob = self.next_knob;
                self.next_knob.0 += 1;
                self.knobs.insert(
                    RegisterKnob {
                        link_idx: self.current_link_id,
                        register,
                    },
                    knob,
                );
                knob
            }
        }

        let mut functions = vec![];
        while let Some(top) = monomorphic.stack.pop() {
            let CommandMonomorphization {
                link_idx,
                command,
                tys,
            } = top;

            monomorphic.current_link_id = link_idx;
            let links = links.get(link_idx).copied().unwrap_or_default();

            let linked = Self::link_in(command, tys, &mut high_ops, &mut monomorphic, links)?;
            // FIXME: expand further requested generic instantiations.
            functions.push(linked);
        }

        Ok(Program {
            ops: high_ops,
            functions,
            entry_index: 0,
            buffer_by_op: HashMap::default(),
            texture_by_op: HashMap::default(),
            knobs: monomorphic.knobs,
        })
    }

    fn link_in(
        command: &Self,
        tys: Cow<'_, [Descriptor]>,
        high_ops: &mut Vec<High>,
        mono: &mut Monomorphizing,
        functions: &[usize],
    ) -> Result<FunctionLinked, CompileError> {
        if functions.len() != command.symbols.len() {
            eprintln!("Bad linked parameter count");
            return Err(CompileError::NotYetImplemented);
        }

        if tys.len() != command.vars.len() {
            eprintln!("Bad type generic count");
            return Err(CompileError::NotYetImplemented);
        }

        let ops = &command.ops;
        let steps = ops.len();
        let tys = tys.as_ref();
        let start = high_ops.len();

        let mut last_use = vec![0; steps];
        let mut first_use = vec![steps; steps];

        let image_buffers = core::cell::RefCell::new(ImageBufferPlan::default());

        // Liveness analysis.
        for (back_idx, op) in ops.iter().rev().enumerate() {
            let idx = ops.len() - 1 - back_idx;
            match op {
                Op::Input { .. }
                | Op::Construct { .. }
                | Op::BufferInit { .. }
                | Op::DynamicImage {
                    call: OperandDynKind::Construct,
                    ..
                } => {}
                &Op::Output { src: Register(src) } => {
                    last_use[src] = last_use[src].max(idx);
                    first_use[src] = first_use[src].min(idx);
                }
                &Op::Render { src: Register(src) } => {
                    last_use[src] = last_use[src].max(idx);
                    first_use[src] = first_use[src].min(idx);
                }
                &Op::Unary {
                    src: Register(src), ..
                }
                | &Op::DynamicImage {
                    call: OperandDynKind::Unary(Register(src)),
                    ..
                }
                | &Op::BufferUnary {
                    src: Register(src), ..
                } => {
                    last_use[src] = last_use[src].max(idx);
                    first_use[src] = first_use[src].min(idx);
                }
                &Op::Binary {
                    lhs: Register(lhs),
                    rhs: Register(rhs),
                    ..
                }
                | &Op::BufferBinary {
                    lhs: Register(lhs),
                    rhs: Register(rhs),
                    ..
                }
                | &Op::DynamicImage {
                    call:
                        OperandDynKind::Binary {
                            lhs: Register(lhs),
                            rhs: Register(rhs),
                        },
                    ..
                } => {
                    last_use[rhs] = last_use[rhs].max(idx);
                    first_use[rhs] = first_use[rhs].min(idx);
                    last_use[lhs] = last_use[lhs].max(idx);
                    first_use[lhs] = first_use[lhs].min(idx);
                }
                Op::Invoke {
                    function: _,
                    arguments: args,
                    results: _,
                    generics: _,
                } => {
                    for &Register(arg) in args {
                        last_use[arg] = last_use[arg].max(idx);
                        first_use[arg] = first_use[arg].min(idx);
                    }
                }
                // Not a use of the return value itself.
                &Op::InvokedResult {
                    invocation: Register(invocation),
                    ..
                } => {
                    last_use[invocation] = last_use[invocation].max(idx);
                    first_use[invocation] = first_use[invocation].min(idx);
                }
            }
        }

        let mut reg_to_texture: HashMap<Register, Texture> = HashMap::default();

        let mut signature_in: Vec<Register> = vec![];
        let mut signature_out: Vec<Register> = vec![];

        let realize_texture = |idx, op: &Op| {
            let liveness = first_use[idx]..last_use[idx];

            // FIXME: not all our High ops actually allocate textures..
            let descriptor = command
                .describe_reg(if let Op::Output { src } = op {
                    *src
                } else if let Op::Render { src } = op {
                    *src
                } else {
                    Register(idx)
                })
                .as_texture()
                .expect("A texture register");

            let descriptor = descriptor.monomorphize(tys);

            let ImageBufferAssignment { buffer: _, texture } = image_buffers
                .borrow_mut()
                .alloc_texture_for(&descriptor, liveness, Register(idx));

            Ok(texture)
        };

        let realize_buffer = |idx, op: &Op| {
            let liveness = first_use[idx]..last_use[idx];

            let descriptor = command
                .describe_reg(if let Op::Output { src } = op {
                    *src
                } else if let Op::Render { src } = op {
                    *src
                } else {
                    Register(idx)
                })
                .as_buffer()
                .expect("A buffer register");

            let len = descriptor.monomorphize(tys);
            let ByteBufferAssignment { buffer } =
                image_buffers
                    .borrow_mut()
                    .alloc_buffer_for(len, liveness, Register(idx));

            Ok(buffer)
        };

        for (idx, op) in ops.iter().enumerate() {
            high_ops.push(High::StackPush(Frame {
                name: format!("Command: {:#?}", op),
            }));

            let idx_reg = Register(idx);

            let knob = match command.knobs.get(&idx_reg) {
                Some(KnobKind::Runtime) => KnobUser::Runtime(mono.next_knob(idx_reg)),
                Some(KnobKind::Buffer { buffer, range }) => {
                    let byte_assignment =
                        match image_buffers.borrow().get_register_resources(*buffer) {
                            Ok(RegisterAssignment::Buffer(buffer)) => buffer,
                            _ => return Err(CompileError::NotYetImplemented),
                        };

                    KnobUser::Buffer {
                        buffer: byte_assignment.buffer,
                        range: range.clone(),
                    }
                }
                None => KnobUser::None,
            };

            match op {
                Op::Input { desc: _ } => {
                    // This implicitly also persists the descriptor
                    let texture = realize_texture(idx, op)?;
                    high_ops.push(High::Input(idx_reg));
                    reg_to_texture.insert(idx_reg, texture);
                    signature_in.push(idx_reg);
                }
                &Op::Output { src } => {
                    let _texture = realize_texture(idx, op)?;
                    signature_out.push(idx_reg);

                    high_ops.push(High::Output {
                        src,
                        dst: Register(idx),
                    });
                }
                &Op::Render { src } => {
                    let _texture = realize_texture(idx, op)?;

                    high_ops.push(High::Render {
                        src,
                        dst: Register(idx),
                    });
                }
                Op::Construct {
                    desc: _,
                    op: construct_op,
                } => {
                    let texture = realize_texture(idx, op)?;

                    match construct_op {
                        &ConstructOp::DistributionNormal(ref distribution) => {
                            high_ops.push(High::DrawInto {
                                dst: Target::Discard(texture),
                                fn_: Initializer::PaintFullScreen {
                                    shader: ParameterizedFragment {
                                        invocation: FragmentShaderInvocation::Normal2d(
                                            distribution.clone(),
                                        ),
                                        knob,
                                    },
                                },
                            })
                        }
                        &ConstructOp::FromBuffer(src) => {
                            // Well we realized the texture, now just initialize it.
                            high_ops.push(High::Copy { src, dst: idx_reg });
                        }
                        ConstructOp::DistributionNoise(ref noise_params) => {
                            high_ops.push(High::DrawInto {
                                dst: Target::Discard(texture),
                                fn_: Initializer::PaintFullScreen {
                                    shader: ParameterizedFragment {
                                        invocation: FragmentShaderInvocation::FractalNoise(
                                            noise_params.clone(),
                                        ),
                                        knob,
                                    },
                                },
                            })
                        }
                        ConstructOp::Bilinear(bilinear) => high_ops.push(High::DrawInto {
                            dst: Target::Discard(texture),
                            fn_: Initializer::PaintFullScreen {
                                shader: ParameterizedFragment {
                                    invocation: FragmentShaderInvocation::Bilinear(
                                        bilinear.clone(),
                                    ),
                                    knob,
                                },
                            },
                        }),
                        &ConstructOp::Solid(color) => high_ops.push(High::DrawInto {
                            dst: Target::Discard(texture),
                            fn_: Initializer::PaintFullScreen {
                                shader: ParameterizedFragment {
                                    invocation: FragmentShaderInvocation::SolidRgb(color.into()),
                                    knob,
                                },
                            },
                        }),
                    }

                    reg_to_texture.insert(idx_reg, texture);
                }
                Op::BufferInit {
                    op: buf_op,
                    desc: _,
                } => {
                    let buffer = realize_buffer(idx, op)?;

                    match buf_op {
                        BufferInitOp::FromData { placement, data } => {
                            high_ops.push(High::WriteInto {
                                dst: buffer,
                                fn_: BufferWrite::Zero,
                            });

                            high_ops.push(High::WriteInto {
                                dst: buffer,
                                fn_: BufferWrite::Put {
                                    placement: placement.clone(),
                                    data: data.clone(),
                                    knob: match knob {
                                        KnobUser::None => None,
                                        KnobUser::Runtime(idx) => Some(idx),
                                        _ => unreachable!(
                                            "Buffer init from buffer does not make sense"
                                        ),
                                    },
                                },
                            });
                        }
                    }
                }
                Op::Unary {
                    desc: _,
                    src,
                    op: unary_op,
                } => {
                    let texture = realize_texture(idx, op)?;

                    match unary_op {
                        &UnaryOp::Crop(region) => {
                            let target =
                                Rectangle::with_width_height(region.width(), region.height());
                            high_ops.push(High::PushOperand(reg_to_texture[src]));
                            high_ops.push(High::DrawInto {
                                dst: Target::Discard(texture),
                                fn_: Initializer::PaintToSelection {
                                    texture: reg_to_texture[src],
                                    selection: region,
                                    target: target.into(),
                                    viewport: target,
                                    shader: ParameterizedFragment {
                                        invocation: FragmentShaderInvocation::PaintOnTop(
                                            PaintOnTopKind::Copy,
                                        ),
                                        knob,
                                    },
                                },
                            });
                        }
                        UnaryOp::ChromaticAdaptation(adaptation) => {
                            // Determine matrix for converting to xyz, then adapt, then back.
                            let adapt = RowMatrix::new(adaptation.to_matrix()?);
                            let output = adapt.multiply_right(adaptation.to_xyz_matrix.into());
                            let matrix = adaptation.from_xyz_matrix.multiply_right(output);

                            high_ops.push(High::PushOperand(reg_to_texture[src]));
                            high_ops.push(High::DrawInto {
                                dst: Target::Discard(texture),
                                fn_: Initializer::PaintFullScreen {
                                    shader: ParameterizedFragment {
                                        invocation: FragmentShaderInvocation::LinearColorMatrix(
                                            shaders::LinearColorTransform {
                                                matrix: matrix.into(),
                                            },
                                        ),
                                        knob,
                                    },
                                },
                            });
                        }
                        UnaryOp::Vignette(vignette) => {
                            todo!()
                        }
                        UnaryOp::ColorConvert(color) => {
                            // The inherent OptoToLinear transformation gets us to a linear light
                            // representation. We want to convert this into a compatible (that is,
                            // using the same observer definition) other linear light
                            // representation that we then transfer back to an electrical form.
                            // Note that these two steps happen, conveniently, automatically.
                            // Usually it is ensured that only two images with the same linear
                            // light representation are used in a single paint call but this
                            // violates it on purpose.

                            high_ops.push(High::PushOperand(reg_to_texture[src]));
                            // FIXME: using a copy here but this means we do this in unnecessarily
                            // many steps. We first decode to linear color, then draw, then code
                            // back to the non-linear electrical space.
                            // We could do this directly from one matrix to another or try using an
                            // ephemeral intermediate attachment?
                            high_ops.push(High::DrawInto {
                                dst: Target::Discard(texture),
                                fn_: Initializer::PaintFullScreen {
                                    shader: ParameterizedFragment {
                                        invocation: color.to_shader(),
                                        knob,
                                    },
                                },
                            });
                        }
                        UnaryOp::Extract { channel: _ } => {
                            high_ops.push(High::PushOperand(reg_to_texture[src]));

                            high_ops.push(High::DrawInto {
                                dst: Target::Discard(texture),
                                fn_: Initializer::PaintFullScreen {
                                    // This will grab the right channel, that is all of them.
                                    // The actual conversion is done in de-staging of the result.
                                    // TODO: evaluate if this is the right way to do it. We could
                                    // also perform a LinearColorMatrix shader here with close to
                                    // the same amount of shader code but a precise result.
                                    shader: ParameterizedFragment {
                                        invocation: FragmentShaderInvocation::PaintOnTop(
                                            PaintOnTopKind::Copy,
                                        ),
                                        knob,
                                    },
                                },
                            })
                        }
                        UnaryOp::Derivative(derivative) => {
                            let invocation = derivative.method.to_shader(derivative.direction)?;

                            high_ops.push(High::PushOperand(reg_to_texture[src]));
                            high_ops.push(High::DrawInto {
                                dst: Target::Discard(texture),
                                fn_: Initializer::PaintFullScreen {
                                    shader: ParameterizedFragment { invocation, knob },
                                },
                            })
                        }
                        UnaryOp::Transmute => high_ops.push(High::Copy {
                            src: *src,
                            dst: Register(idx),
                        }),
                    }

                    reg_to_texture.insert(Register(idx), texture);
                }
                Op::Binary {
                    desc: _,
                    lhs,
                    rhs,
                    op: binary_op,
                } => {
                    let texture = realize_texture(idx, op)?;

                    let lhs_descriptor = command
                        .describe_reg(*lhs)
                        .as_texture()
                        .unwrap()
                        .monomorphize(tys);

                    let rhs_descriptor = command
                        .describe_reg(*rhs)
                        .as_texture()
                        .unwrap()
                        .monomorphize(tys);

                    let lower_region = Rectangle::from(&lhs_descriptor);
                    let upper_region = Rectangle::from(&rhs_descriptor);

                    match binary_op {
                        BinaryOp::Affine(affine) => {
                            let affine_matrix = RowMatrix::new(affine.transformation);

                            high_ops.push(High::PushOperand(reg_to_texture[lhs]));
                            high_ops.push(High::DrawInto {
                                dst: Target::Discard(texture),
                                fn_: Initializer::PaintToSelection {
                                    texture: reg_to_texture[lhs],
                                    selection: lower_region,
                                    target: lower_region.into(),
                                    viewport: lower_region,
                                    shader: ParameterizedFragment {
                                        invocation: FragmentShaderInvocation::PaintOnTop(
                                            PaintOnTopKind::Copy,
                                        ),
                                        knob: knob.clone(),
                                    },
                                },
                            });

                            high_ops.push(High::PushOperand(reg_to_texture[rhs]));
                            high_ops.push(High::DrawInto {
                                dst: Target::Load(texture),
                                fn_: Initializer::PaintToSelection {
                                    texture: reg_to_texture[rhs],
                                    selection: upper_region,
                                    target: QuadTarget::from(upper_region).affine(&affine_matrix),
                                    viewport: lower_region,
                                    shader: ParameterizedFragment {
                                        invocation: FragmentShaderInvocation::PaintOnTop(
                                            affine.sampling.as_paint_on_top()?,
                                        ),
                                        knob,
                                    },
                                },
                            })
                        }
                        BinaryOp::Inject {
                            channel,
                            from_channels,
                        } => {
                            high_ops.push(High::PushOperand(reg_to_texture[lhs]));
                            high_ops.push(High::PushOperand(reg_to_texture[rhs]));

                            high_ops.push(High::DrawInto {
                                dst: Target::Discard(texture),
                                fn_: Initializer::PaintFullScreen {
                                    shader: ParameterizedFragment {
                                        invocation: FragmentShaderInvocation::Inject(
                                            shaders::inject::Shader {
                                                mix: channel.into_vec4(),
                                                color: from_channels.channel_weight_vec4().unwrap(),
                                            },
                                        ),
                                        knob,
                                    },
                                },
                            })
                        }
                        BinaryOp::Inscribe { placement } => {
                            high_ops.push(High::PushOperand(reg_to_texture[lhs]));
                            high_ops.push(High::DrawInto {
                                dst: Target::Discard(texture),
                                fn_: Initializer::PaintToSelection {
                                    texture: reg_to_texture[lhs],
                                    selection: lower_region,
                                    target: lower_region.into(),
                                    viewport: lower_region,
                                    shader: ParameterizedFragment {
                                        invocation: FragmentShaderInvocation::PaintOnTop(
                                            PaintOnTopKind::Copy,
                                        ),
                                        knob: knob.clone(),
                                    },
                                },
                            });

                            high_ops.push(High::PushOperand(reg_to_texture[rhs]));
                            high_ops.push(High::DrawInto {
                                dst: Target::Load(texture),
                                fn_: Initializer::PaintToSelection {
                                    texture: reg_to_texture[rhs],
                                    selection: upper_region,
                                    target: (*placement).into(),
                                    viewport: lower_region,
                                    shader: ParameterizedFragment {
                                        invocation: FragmentShaderInvocation::PaintOnTop(
                                            PaintOnTopKind::Copy,
                                        ),
                                        knob,
                                    },
                                },
                            });
                        }
                        BinaryOp::Palette(shader) => {
                            high_ops.push(High::PushOperand(reg_to_texture[lhs]));
                            high_ops.push(High::PushOperand(reg_to_texture[rhs]));

                            high_ops.push(High::DrawInto {
                                dst: Target::Load(texture),
                                fn_: Initializer::PaintFullScreen {
                                    shader: ParameterizedFragment {
                                        invocation: FragmentShaderInvocation::Palette(
                                            shader.clone(),
                                        ),
                                        knob,
                                    },
                                },
                            });
                        }
                        BinaryOp::GainMap(_) => {
                            todo!()
                        }
                    }

                    reg_to_texture.insert(Register(idx), texture);
                }
                Op::DynamicImage { call, command, .. } => {
                    let texture = realize_texture(idx, op)?;
                    let (op_unary, op_binary, arguments);

                    match call {
                        OperandDynKind::Construct => {
                            arguments = &[][..];
                            reg_to_texture.insert(Register(idx), texture);
                        }
                        OperandDynKind::Unary(reg) => {
                            op_unary = [reg_to_texture[reg]];
                            arguments = &op_unary[..];
                            reg_to_texture.insert(Register(idx), texture);
                        }
                        OperandDynKind::Binary { lhs, rhs } => {
                            op_binary = [reg_to_texture[lhs], reg_to_texture[rhs]];
                            arguments = &op_binary[..];
                            reg_to_texture.insert(Register(idx), texture);
                        }
                    }

                    if command.num_args != arguments.len() as u32 {
                        // FIXME: should just error with information. We need to pin-point  the
                        // source of the num args to either the library (an internal bug) or the
                        // user for dynamically constructed shaders. Also consider if the number of
                        // arguments can be recovered from the SPIR-V earlier.
                        return Err(CompileError::NotYetImplemented);
                    }

                    for &operand in arguments {
                        high_ops.push(High::PushOperand(operand));
                    }

                    // This always 'constructs' an output texture. The image we render to is new,
                    // no matter how many arguments are being inserted.
                    high_ops.push(High::DrawInto {
                        dst: Target::Discard(texture),
                        fn_: Initializer::PaintFullScreen {
                            shader: ParameterizedFragment {
                                invocation: FragmentShaderInvocation::Runtime(command.clone()),
                                knob,
                            },
                        },
                    })
                }
                Op::InvokedResult { .. } => {
                    let texture = realize_texture(idx, op)?;

                    high_ops.push(High::Uninit {
                        dst: Target::Discard(texture),
                    });

                    reg_to_texture.insert(Register(idx), texture);
                }
                Op::Invoke {
                    function,
                    arguments,
                    results,
                    generics,
                } => {
                    let monomorphic_tys: Vec<_> = generics
                        .iter()
                        .map(|gen| gen.monomorphize(tys))
                        .collect::<_>();

                    let &FunctionVar(function_idx) = function;
                    let link_idx = *functions
                        .get(function_idx)
                        .ok_or(CompileError::NotYetImplemented)?;

                    let function = mono.push_function(LinkedMonomorphicSignature {
                        link_idx,
                        tys: monomorphic_tys,
                    });

                    let mut image_io = vec![];

                    for &register in arguments {
                        // Arguments must precede the function and already be laid out.
                        if register.0 >= idx {
                            return Err(CompileError::NotYetImplemented);
                        }

                        let texture = realize_texture(register.0, &ops[register.0])?;
                        image_io.push(CallBinding::InTexture { register, texture });
                    }

                    for &register in results {
                        // Results must precede the function and already be laid out. They are not
                        // initialized but initialized on return.
                        if register.0 >= idx {
                            return Err(CompileError::NotYetImplemented);
                        }

                        let texture = realize_texture(register.0, &ops[register.0])?;
                        image_io.push(CallBinding::OutTexture { register, texture });
                    }

                    high_ops.push(High::Call {
                        function,
                        image_io_buffers: Arc::from(image_io),
                    });
                }
                // In case we add a new case.
                #[allow(unreachable_patterns)]
                _ => {
                    eprintln!("Unimplemented operation");
                    return Err(CompileError::NotYetImplemented);
                }
            }

            high_ops.push(High::Done(Register(idx)));
            high_ops.push(High::StackPop);
        }

        let end = high_ops.len();

        // The registers which callers must fill. This must match the order that CallBinding is
        // passed at call sites, i.e. be consistent with the signature.
        let signature_registers = signature_in.into_iter().chain(signature_out).collect();

        Ok(FunctionLinked {
            ops: start..end,
            image_buffers: image_buffers.into_inner(),
            signature_registers,
        })
    }

    /// Get the descriptor for a register.
    fn describe_reg(&self, Register(reg): Register) -> RegisterDescription<'_> {
        match self.ops.get(reg) {
            None | Some(Op::Output { .. }) | Some(Op::Render { .. }) => RegisterDescription::None,
            Some(Op::Invoke { .. }) => {
                // This does not describe results directly.
                RegisterDescription::None
            }
            Some(Op::InvokedResult { desc, .. })
            | Some(Op::Input { desc })
            | Some(Op::Construct { desc, .. })
            | Some(Op::Unary { desc, .. })
            | Some(Op::Binary { desc, .. })
            | Some(Op::DynamicImage { desc, .. }) => RegisterDescription::Texture(desc),
            Some(Op::BufferInit { desc, .. })
            | Some(Op::BufferUnary { desc, .. })
            | Some(Op::BufferBinary { desc, .. }) => RegisterDescription::Buffer(desc),
        }
    }

    fn push(&mut self, op: Op) -> Register {
        let reg = Register(self.ops.len());
        self.ops.push(op);
        reg
    }
}

/// Impls on `CommandBuffer` that allow defining custom SPIR-V extensions.
///
/// Generally, the steps on the dynamic shader are:
///
/// 1. Check the kind, get SPIR-v code.
/// 2. Determine the dynamic typing of the result.
/// 3. Have the shader create binary representation of its data.
/// 3. Create a new entry on the command buffer.
/// 4. Not yet performed: (Validate the SPIR-V module inputs against the data definition)
impl CommandBuffer {
    /// Record a _constructor_, with a user-supplied shader.
    pub fn construct_dynamic(&mut self, dynamic: &dyn ShaderCommand) -> Register {
        let mut data = vec![];
        let mut content = None;

        let source = dynamic.source();
        let desc = dynamic.data(ShaderData {
            data_buffer: &mut data,
            content: &mut content,
        });

        self.push(Op::DynamicImage {
            call: OperandDynKind::Construct,
            // FIXME: maybe this conversion should be delayed.
            // In particular, converting source to SPIR-V may take some form of 'compiler' argument
            // that's only available during `compile` phase.
            command: ShaderInvocation {
                spirv: match source {
                    ShaderSource::SpirV(spirv) => spirv,
                },
                shader_data: match content {
                    None => None,
                    Some(c) => Some(c.as_slice(&data).into()),
                },
                num_args: 0,
            },
            desc: desc.into(),
        })
    }

    /// Record a unary operator, with a user-supplied shader.
    pub fn unary_dynamic(
        &mut self,
        op: Register,
        dynamic: &dyn ShaderCommand,
    ) -> Result<Register, CommandError> {
        let _input_descriptor = match self.describe_reg(op) {
            RegisterDescription::Texture(desc) => desc,
            _ => return Err(CommandError::INVALID_CALL),
        };

        let mut data = vec![];
        let mut content = None;

        let source = dynamic.source();
        let desc = dynamic.data(ShaderData {
            data_buffer: &mut data,
            content: &mut content,
        });

        let out_reg = self.push(Op::DynamicImage {
            call: OperandDynKind::Unary(op),
            // FIXME: maybe this conversion should be delayed.
            // In particular, converting source to SPIR-V may take some form of 'compiler' argument
            // that's only available during `compile` phase.
            command: ShaderInvocation {
                spirv: match source {
                    ShaderSource::SpirV(spirv) => spirv,
                },
                shader_data: match content {
                    None => None,
                    Some(c) => Some(c.as_slice(&data).into()),
                },
                num_args: 1,
            },
            desc: desc.into(),
        });

        Ok(out_reg)
    }

    /// Record a binary operator, with a user-supplied shader.
    pub fn binary_dynamic(
        &mut self,
        lhs: Register,
        rhs: Register,
        dynamic: &dyn ShaderCommand,
    ) -> Result<Register, CommandError> {
        let _input_descriptor = match self.describe_reg(lhs) {
            RegisterDescription::Texture(desc) => desc,
            _ => return Err(CommandError::INVALID_CALL),
        };

        let _input_descriptor = match self.describe_reg(rhs) {
            RegisterDescription::Texture(desc) => desc,
            _ => return Err(CommandError::INVALID_CALL),
        };

        let mut data = vec![];
        let mut content = None;

        let source = dynamic.source();
        let desc = dynamic.data(ShaderData {
            data_buffer: &mut data,
            content: &mut content,
        });

        let out_reg = self.push(Op::DynamicImage {
            call: OperandDynKind::Binary { lhs, rhs },
            // FIXME: maybe this conversion should be delayed.
            // In particular, converting source to SPIR-V may take some form of 'compiler' argument
            // that's only available during `compile` phase.
            command: ShaderInvocation {
                spirv: match source {
                    ShaderSource::SpirV(spirv) => spirv,
                },
                shader_data: match content {
                    None => None,
                    Some(c) => Some(c.as_slice(&data).into()),
                },
                num_args: 2,
            },
            desc: desc.into(),
        });

        Ok(out_reg)
    }
}

impl CommandSignature {
    /// Verify if a signature matches an other command signature.
    ///
    /// That is, whether the subtyping relationship of all its bounds and the argument allows using
    /// one in place of the other declared type. This checks if `self` contains all bounds that
    /// occur in `actual`.
    pub fn is_declaration_of(&self, actual: &CommandSignature) -> bool {
        if self.vars.len() != actual.vars.len() {
            return false;
        }

        for (decl, actual) in self.vars.iter().zip(&actual.vars) {
            if !decl.contains_bounds(actual) {
                return false;
            }
        }

        true
    }
}

impl GenericDescriptor {
    /// Query if this describes a monomorphic descriptor.
    ///
    /// At the moment this means a fully constrained descriptor where both size and chroma are
    /// defined. It's a bit odd that this would be an overlapping property with having been
    /// constructed from an actually concrete defined descriptor. If we had a non-deterministic
    /// layout algorithm (i.e. multiple permissible layouts for one combination of size/chroma)
    /// then this might inadvertently throw away some of this information. But for now this
    /// information is compile time only, the actual dependence of operational semantics on layout
    /// information is evaluated at runtime. (FIXME: I will have regretted writing this).
    pub fn as_concrete(&self) -> Option<Descriptor> {
        let Generic::Concrete((w, h)) = self.size else {
            return None;
        };

        let Generic::Concrete((texel, color)) = &self.chroma else {
            return None;
        };

        Descriptor::with_texel(texel.clone(), w, h).map(|mut desc| {
            desc.color = color.clone();
            desc
        })
    }

    /// FIXME: fallible. If we change the texel from something small to something very large we can
    /// exceed the allocation limits that are necessary to express the layout.
    pub fn with_chroma(&self, texel: Texel, color: Color) -> Self {
        GenericDescriptor {
            chroma: Generic::Concrete((texel, color)),
            ..self.clone()
        }
    }

    pub fn monomorphize(&self, decl: &[Descriptor]) -> Descriptor {
        let (w, h) = match &self.size {
            Generic::Concrete(descriptor) => descriptor.clone(),
            Generic::Generic(idx) => decl[idx.0].size(),
        };

        let (texel, color) = match &self.chroma {
            Generic::Concrete(tuple) => tuple.clone(),
            Generic::Generic(idx) => {
                let from = &decl[idx.0];
                (from.texel.clone(), from.color.clone())
            }
        };

        Descriptor::with_texel(texel, w, h)
            .map(|mut desc| {
                desc.color = color;
                desc
            })
            .expect("changing texel and color to something that does not fit memory")
    }

    /// Apply an outer variable definition, replacing generics by at least as concrete terms.
    ///
    /// Does not verify any bounds of the rewrites! Which we'll need to do if we had associated
    /// constants and the rewrite was looking into paths and impls. Consider a trait (similar to
    /// the Rust type system) / type family such as `LinearizedColor` that associates the linear
    /// optical colorspace to an arbitrary electrical color encoding. Then we might have the
    /// signature written in pseudo-code:
    ///
    /// ```text
    ///     function <C: LinearizedColor>(arg0: {C; 256×256}, arg1: {C::Linear; 256×256})
    /// ```
    ///
    /// Now if we rewrite with [C = sRGB] then we want the concrete [C::Linear=CIE-RGB-Wp-D70]
    /// correspondence. But if we tried [C = CYMK] we have nonsense. Here we allow this function to
    /// panic, a check must happen earlier.
    pub fn rewrite(&self, decl: &[GenericDescriptor]) -> Self {
        GenericDescriptor {
            size: match &self.size {
                &Generic::Concrete(size) => Generic::Concrete(size),
                Generic::Generic(idx) => decl[idx.0].size.clone(),
            },
            chroma: match &self.chroma {
                Generic::Concrete(chroma) => Generic::Concrete(chroma.clone()),
                Generic::Generic(idx) => decl[idx.0].chroma.clone(),
            },
        }
    }

    pub fn size(&self) -> Generic<(u32, u32)> {
        self.size.clone()
    }

    pub fn descriptor_chroma(&self) -> Generic<(Texel, Color)> {
        self.chroma.clone()
    }
}

impl GenericBuffer {
    pub fn as_concrete(&self) -> Option<u64> {
        match self.size {
            Generic::Concrete(val) => Some(val),
            Generic::Generic(_) => None,
        }
    }

    pub fn monomorphize(&self, decl: &[Descriptor]) -> u64 {
        match self.size {
            Generic::Concrete(val) => val,
            Generic::Generic(var) => decl[var.0].to_canvas().u64_len(),
        }
    }
}

impl From<Descriptor> for GenericDescriptor {
    fn from(desc: Descriptor) -> Self {
        let size = desc.size();
        let chroma = (desc.texel.clone(), desc.color.clone());

        GenericDescriptor {
            size: Generic::Concrete(size),
            chroma: Generic::Concrete(chroma),
        }
    }
}

impl<'lt> RegisterDescription<'lt> {
    pub fn as_texture(&self) -> Result<&'lt GenericDescriptor, CommandError> {
        match self {
            RegisterDescription::Texture(tex) => Ok(tex),
            _ => Err(CommandError::BAD_REGISTER),
        }
    }

    pub fn as_buffer(&self) -> Result<&'lt GenericBuffer, CommandError> {
        match self {
            RegisterDescription::Buffer(tex) => Ok(tex),
            _ => Err(CommandError::BAD_REGISTER),
        }
    }
}

impl TyVarBounds {
    pub fn contains_bounds(&self, actual: &TyVarBounds) -> bool {
        self.is_empty() && actual.is_empty()
    }

    fn is_empty(&self) -> bool {
        // FIXME: if we collect the list.
        true
    }
}

impl ColorConversion {
    pub(crate) fn to_shader(&self) -> FragmentShaderInvocation {
        match self {
            ColorConversion::Xyz {
                to_xyz_matrix,
                from_xyz_matrix,
            } => {
                let from = from_xyz_matrix.inv();
                let matrix = to_xyz_matrix.multiply_right(from.into()).into();

                FragmentShaderInvocation::LinearColorMatrix(shaders::LinearColorTransform {
                    matrix,
                })
            }
            ColorConversion::XyzToOklab { to_xyz_matrix } => {
                FragmentShaderInvocation::Oklab(shaders::oklab::Shader::with_encode(*to_xyz_matrix))
            }
            ColorConversion::OklabToXyz { from_xyz_matrix } => {
                let from_xyz_matrix = from_xyz_matrix.inv();
                FragmentShaderInvocation::Oklab(shaders::oklab::Shader::with_decode(
                    from_xyz_matrix,
                ))
            }
            ColorConversion::XyzToSrLab2 {
                to_xyz_matrix,
                whitepoint,
            } => FragmentShaderInvocation::SrLab2(shaders::srlab2::Shader::with_encode(
                *to_xyz_matrix,
                *whitepoint,
            )),
            ColorConversion::SrLab2ToXyz {
                from_xyz_matrix,
                whitepoint,
            } => {
                let from_xyz_matrix = from_xyz_matrix.inv();
                FragmentShaderInvocation::SrLab2(shaders::srlab2::Shader::with_decode(
                    from_xyz_matrix,
                    *whitepoint,
                ))
            }
        }
    }
}

impl ChromaticAdaptation {
    pub(crate) fn to_matrix(&self) -> Result<[f32; 9], CompileError> {
        use palette::{
            chromatic_adaptation::{Method, TransformMatrix},
            white_point as wp,
        };

        // FIXME: when you adjust the value-to-type translation, also adjust it within `method`.
        macro_rules! translate_matrix {
            ($source:expr, $target:expr, $($lhs:ident => $lhsty:ty)|*) => {
                $(
                    translate_matrix!(
                        @$source, $target, $lhs => $lhsty :
                        A => wp::A | B => wp::B | C => wp::C
                        | D50 => wp::D50 | D55 => wp::D55 | D65 => wp::D65
                        | D75 => wp::D75 | E => wp::E | F2 => wp::F2
                        | F7 => wp::F7 | F11 => wp::F11
                    );
                )*
            };
            (@$source:expr, $target:expr, $lhs:ident => $lhsty:ty : $($rhs:ident => $ty:ty)|*) => {
                $(
                    if let (Whitepoint::$lhs, Whitepoint::$rhs) = ($source, $target) {
                        return Ok((|method| {
                            let lhswp = <$lhsty as wp::WhitePoint<f32>>::get_xyz();
                            let rhswp = <$ty as wp::WhitePoint<f32>>::get_xyz();
                            <Method as TransformMatrix<f32>>::generate_transform_matrix(method, lhswp, rhswp)
                        })
                                  as fn(&Method) -> [f32;9]);
                    }
                )*
            };
        }

        // FIXME: when you adjust the value-to-type translation, also adjust it within
        // `translate_matrix!`
        let method = (|| {
            translate_matrix! {
                self.source, self.target,
                A => wp::A | B => wp::B | C => wp::C
                | D50 => wp::D50 | D55 => wp::D55 | D65 => wp::D65
                | D75 => wp::D75 | E => wp::E | F2 => wp::F2
                | F7 => wp::F7 | F11 => wp::F11
            };

            Err(CompileError::NotYetImplemented)
        })()?;

        let matrices = method(match self.method {
            // Bradford's original method that does slight blue non-linearity is not yet supported.
            // Please implement the paper if you feel compelled to.
            ChromaticAdaptationMethod::BradfordNonLinear => {
                return Err(CompileError::NotYetImplemented)
            }
            ChromaticAdaptationMethod::BradfordVonKries => &Method::Bradford,
            ChromaticAdaptationMethod::VonKries => &Method::VonKries,
            ChromaticAdaptationMethod::Xyz => &Method::XyzScaling,
        });

        Ok(matrices)
    }
}

#[rustfmt::skip]
impl DerivativeMethod {
    fn to_shader(&self, direction: Direction) -> Result<FragmentShaderInvocation, CompileError> {
        use DerivativeMethod::*;
        use shaders::box3;
        match self {
            Prewitt => {
                let matrix = RowMatrix::with_outer_product(
                    [1./3., 1./3., 1./3.],
                    [0.5, 0.0, -0.5],
                );

                let shader = box3::Shader::new(direction.adjust_vertical_box(matrix));
                Ok(shaders::FragmentShaderInvocation::Box3(shader))
            }
            Sobel => {
                let matrix = RowMatrix::with_outer_product(
                    [1./4., 1./2., 1./4.],
                    [0.5, 0.0, -0.5],
                );

                let shader = box3::Shader::new(direction.adjust_vertical_box(matrix));
                Ok(shaders::FragmentShaderInvocation::Box3(shader))
            }
            Scharr3 => {
                let matrix = RowMatrix::with_outer_product(
                    [46.84/256., 162.32/256., 46.84/256.],
                    [0.5, 0.0, -0.5],
                );

                let shader = box3::Shader::new(direction.adjust_vertical_box(matrix));
                Ok(shaders::FragmentShaderInvocation::Box3(shader))
            }
            Scharr3To4Bit => {
                let matrix = RowMatrix::with_outer_product(
                    [3./16., 10./16., 3./16.],
                    [0.5, 0.0, -0.5],
                );

                let shader = box3::Shader::new(direction.adjust_vertical_box(matrix));
                Ok(shaders::FragmentShaderInvocation::Box3(shader))
            }
            Scharr3To8Bit => {
                let matrix = RowMatrix::with_outer_product(
                    [47./256., 162./256., 47./256.],
                    [0.5, 0.0, -0.5],
                );

                let shader = box3::Shader::new(direction.adjust_vertical_box(matrix));
                Ok(shaders::FragmentShaderInvocation::Box3(shader))
            }
            // FIXME: implement these.
            // When you do add them to tests/blend.rs
            | Roberts
            | Scharr4
            | Scharr5
            | Scharr5Tab => Err(CompileError::NotYetImplemented)
        }
    }
}

impl Direction {
    fn adjust_vertical_box(self, mat: RowMatrix) -> RowMatrix {
        match self {
            Direction::Width => mat,
            Direction::Height => mat.transpose(),
        }
    }
}

#[rustfmt::skip]
impl Affine {
    /// Create affine parameters with identity transformation.
    pub fn new(sampling: AffineSample) -> Self {
        Affine {
            transformation: [
                1.0, 0., 0.,
                0., 1.0, 0.,
                0., 0., 1.0,
            ],
            sampling,
        }
    }

    /// After the transformation, also scale everything.
    ///
    /// This corresponds to a left-side multiplication of the transformation matrix.
    pub fn scale(self, x: f32, y: f32) -> Self {
        let post = RowMatrix::diag(x, y, 1.0)
            .multiply_right(RowMatrix::new(self.transformation).into());
        let transformation = RowMatrix::from(post).into_inner();

        Affine {
            transformation,
            ..self
        }
    }

    /// After the transformation, rotate everything clockwise.
    ///
    /// This corresponds to a left-side multiplication of the transformation matrix.
    pub fn rotate(self, rad: f32) -> Self {
        let post = RowMatrix::new([
            rad.cos(), rad.sin(), 0.,
            -rad.sin(), rad.cos(), 0.,
            0., 0., 1.,
        ]);

        let post = post.multiply_right(RowMatrix::new(self.transformation).into());
        let transformation = RowMatrix::from(post).into_inner();

        Affine {
            transformation,
            ..self
        }
    }

    /// After the transformation, shift by an x and y offset.
    ///
    /// This corresponds to a left-side multiplication of the transformation matrix.
    pub fn shift(self, x: f32, y: f32) -> Self {
        let post = RowMatrix::new([
            1., 0., x,
            0., 1., y,
            0., 0., 1.,
        ]);

        let post = post.multiply_right(RowMatrix::new(self.transformation).into());
        let transformation = RowMatrix::from(post).into_inner();

        Affine {
            transformation,
            ..self
        }
    }
}

impl AffineSample {
    fn as_paint_on_top(self) -> Result<PaintOnTopKind, CompileError> {
        match self {
            AffineSample::Nearest => Ok(PaintOnTopKind::Copy),
            _ => Err(CompileError::NotYetImplemented),
        }
    }
}

impl Rectangle {
    /// A rectangle at the origin with given width (x) and height (y).
    pub fn with_width_height(width: u32, height: u32) -> Self {
        Rectangle {
            x: 0,
            y: 0,
            max_x: width,
            max_y: height,
        }
    }

    /// A rectangle describing a complete buffer.
    pub fn with_layout(buffer: &ByteLayout) -> Self {
        Self::with_width_height(buffer.width, buffer.height)
    }

    /// The apparent width.
    pub fn width(self) -> u32 {
        self.max_x.saturating_sub(self.x)
    }

    /// The apparent height.
    pub fn height(self) -> u32 {
        self.max_y.saturating_sub(self.y)
    }

    /// Return true if this rectangle fully contains `other`.
    pub fn contains(self, other: Self) -> bool {
        self.x <= other.x && self.y <= other.y && {
            // Offsets are surely non-wrapping.
            let offset_x = other.x - self.x;
            let offset_y = other.y - self.y;
            let rel_width = self.width().checked_sub(offset_x);
            let rel_height = self.height().checked_sub(offset_y);
            rel_width >= Some(other.width()) && rel_height >= Some(other.height())
        }
    }

    /// Bring the rectangle into normalized form where minimum and maximum form a true interval.
    #[must_use]
    pub fn normalize(self) -> Rectangle {
        Rectangle {
            x: self.x,
            y: self.y,
            max_x: self.x + self.width(),
            max_y: self.y + self.width(),
        }
    }

    /// A rectangle that the overlap of the two.
    #[must_use]
    pub fn meet(self, other: Self) -> Rectangle {
        Rectangle {
            x: self.x.max(other.x),
            y: self.y.max(other.y),
            max_x: self.max_x.min(other.max_x),
            max_y: self.max_y.min(other.max_y),
        }
    }

    /// The meet, relative to the coordinates of this rectangle.
    #[must_use]
    pub fn meet_in_local_coordinates(self, other: Self) -> Rectangle {
        // Normalize to ensure that max_{x,y} is not less than {x,y}
        let meet = self.normalize().meet(other);
        Rectangle {
            x: meet.x - self.x,
            y: meet.y - self.y,
            max_x: meet.max_x - self.x,
            max_y: meet.max_y - self.y,
        }
    }

    /// A rectangle that contains both.
    #[must_use]
    pub fn join(self, other: Self) -> Rectangle {
        Rectangle {
            x: self.x.min(other.x),
            y: self.y.min(other.y),
            max_x: self.max_x.max(other.max_x),
            max_y: self.max_y.max(other.max_y),
        }
    }

    /// Remove border from all sides.
    /// When the image is smaller than `border` in some dimension then the result is empty and
    /// contained in the original image but otherwise unspecified.
    #[must_use]
    pub fn inset(self, border: u32) -> Self {
        Rectangle {
            x: self.x.saturating_add(border),
            y: self.y.saturating_add(border),
            max_x: self.max_x.saturating_sub(border),
            max_y: self.max_y.saturating_sub(border),
        }
    }
}

impl From<&'_ ByteLayout> for Rectangle {
    fn from(buffer: &ByteLayout) -> Rectangle {
        Rectangle::with_width_height(buffer.width, buffer.height)
    }
}

impl From<&'_ CanvasLayout> for Rectangle {
    fn from(buffer: &CanvasLayout) -> Rectangle {
        Rectangle::with_width_height(buffer.width(), buffer.height())
    }
}

impl From<&'_ Descriptor> for Rectangle {
    fn from(buffer: &Descriptor) -> Rectangle {
        Rectangle::from(&buffer.layout)
    }
}

impl CommandError {
    /// Indicates a very generic type error.
    const TYPE_ERR: Self = CommandError {
        inner: CommandErrorKind::GenericTypeError,
    };

    /// Indicates a very generic other error.
    /// E.g. the usage of a command requires an extension? Not quite sure yet.
    const OTHER: Self = CommandError {
        inner: CommandErrorKind::Other,
    };

    /// Specifies that a register reference was invalid.
    const BAD_REGISTER: Self = Self::OTHER;

    /// Specifies that a register reference was invalid.
    const INVALID_CALL: Self = Self::OTHER;

    /// This has not yet been implemented, sorry.
    ///
    /// Errors of this kind will be removed over the course of bringing the crate to a first stable
    /// release, this this will be removed. The method, and importantly its signature, are already
    /// added for the purpose of exposition and documenting the intention.
    const UNIMPLEMENTED: Self = CommandError {
        inner: CommandErrorKind::Unimplemented,
    };

    pub fn is_type_err(&self) -> bool {
        matches!(
            self.inner,
            CommandErrorKind::GenericTypeError
                | CommandErrorKind::ConflictingTypes(_, _)
                | CommandErrorKind::BadDescriptor(_, _)
        )
    }
}

#[test]
fn rectangles() {
    let small = Rectangle::with_width_height(2, 2);
    let large = Rectangle::with_width_height(4, 4);

    assert_eq!(large, large.join(small));
    assert_eq!(small, large.meet(small));
    assert!(large.contains(small));
    assert!(!small.contains(large));
}

#[test]
fn simple_program() {
    use crate::pool::Pool;

    const BACKGROUND: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/input/background.png");
    const FOREGROUND: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/input/foreground.png");

    let mut pool = Pool::new();
    let mut commands = CommandBuffer::default();

    let background = image::open(BACKGROUND).expect("Background image opened");
    let foreground = image::open(FOREGROUND).expect("Background image opened");
    let expected = ByteLayout::from(&background);

    let placement = Rectangle {
        x: 0,
        y: 0,
        max_x: foreground.width(),
        max_y: foreground.height(),
    };

    let background = pool.insert_srgb(&background);
    let background = commands.input_from(background.into());

    let foreground = pool.insert_srgb(&foreground);
    let foreground = commands.input_from(foreground.into());

    let result = commands
        .inscribe(background, placement, foreground)
        .expect("Valid to inscribe");
    let (_, outformat) = commands.output(result).expect("Valid for output");

    let _ = commands.compile().expect("Could build command buffer");
    assert_eq!(outformat.as_concrete().map(|x| x.layout), Some(expected));
}
