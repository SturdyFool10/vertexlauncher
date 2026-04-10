//! HDR rendering configuration with FP16/FP32 precision and multiple colorspaces.

/// Internal buffer precision for HDR rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferPrecision {
    /// 16-bit floating point (half precision).
    FP16,
    /// 32-bit floating point (full precision).
    FP32,
}

impl Default for BufferPrecision {
    fn default() -> Self {
        BufferPrecision::FP16
    }
}

impl BufferPrecision {
    /// Returns the wgpu texture format for color buffers with this precision.
    pub fn rgba_format(&self) -> wgpu::TextureFormat {
        match self {
            BufferPrecision::FP16 => wgpu::TextureFormat::Rgba16Float,
            BufferPrecision::FP32 => wgpu::TextureFormat::Rgba32Float,
        }
    }

    /// Returns the wgpu texture format for RG color buffers.
    pub fn rg_format(&self) -> wgpu::TextureFormat {
        match self {
            BufferPrecision::FP16 => wgpu::TextureFormat::Rg16Float,
            BufferPrecision::FP32 => wgpu::TextureFormat::Rg32Float,
        }
    }

    /// Returns the wgpu texture format for R color buffers.
    pub fn r_format(&self) -> wgpu::TextureFormat {
        match self {
            BufferPrecision::FP16 => wgpu::TextureFormat::R16Float,
            BufferPrecision::FP32 => wgpu::TextureFormat::R32Float,
        }
    }

    /// Returns the byte size per channel.
    pub fn bytes_per_channel(&self) -> usize {
        match self {
            BufferPrecision::FP16 => 2,
            BufferPrecision::FP32 => 4,
        }
    }
}

/// Output colorspace configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Colorspace {
    /// Standard Dynamic Range (SDR) - sRGB.
    SDR,
    /// HDR10 / ST2084 PQ curve.
    HDR10,
    /// Dolby Vision (requires additional metadata).
    DolbyVision,
    /// Linear colorspace (no tone mapping).
    Linear,
}

impl Default for Colorspace {
    fn default() -> Self {
        Colorspace::SDR
    }
}

impl Colorspace {
    /// Returns true if this is an HDR colorspace.
    pub fn is_hdr(&self) -> bool {
        matches!(
            self,
            Colorspace::HDR10 | Colorspace::DolbyVision | Colorspace::Linear
        )
    }

    /// Returns the maximum theoretical brightness in nits for this colorspace.
    pub fn max_theoretical_brightness(&self) -> u32 {
        match self {
            Colorspace::SDR => 100,
            Colorspace::HDR10 => 10_000,
            Colorspace::DolbyVision => 4_000,
            Colorspace::Linear => 100_000, // Theoretically unlimited
        }
    }

    /// Returns the wgpu texture format for final output.
    pub fn output_format(&self) -> wgpu::TextureFormat {
        match self {
            Colorspace::SDR => wgpu::TextureFormat::Bgra8UnormSrgb,
            Colorspace::HDR10 | Colorspace::DolbyVision => wgpu::TextureFormat::Rgba16Float,
            Colorspace::Linear => wgpu::TextureFormat::Rgba32Float,
        }
    }
}

/// HDR rendering configuration.
#[derive(Debug, Clone)]
pub struct HdrConfig {
    /// Internal buffer precision for render targets.
    pub internal_precision: BufferPrecision,
    /// Output colorspace/tone mapping curve.
    pub output_colorspace: Colorspace,
    /// Maximum display brightness in nits (for HDR10).
    pub max_brightness_nits: u32,
    /// Minimum display brightness in nits (for HDR10).
    pub min_brightness_nits: f64,
}

impl Default for HdrConfig {
    fn default() -> Self {
        Self {
            internal_precision: BufferPrecision::FP16,
            output_colorspace: Colorspace::SDR,
            max_brightness_nits: 1000,
            min_brightness_nits: 0.1,
        }
    }
}

impl HdrConfig {
    /// Create a new HDR configuration with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create SDR configuration (standard dynamic range).
    pub fn sdr() -> Self {
        Self {
            internal_precision: BufferPrecision::FP16,
            output_colorspace: Colorspace::SDR,
            max_brightness_nits: 100,
            min_brightness_nits: 0.0,
        }
    }

    /// Create HDR10 configuration with specified brightness range.
    pub fn hdr10(max_nits: u32) -> Self {
        Self {
            internal_precision: BufferPrecision::FP16,
            output_colorspace: Colorspace::HDR10,
            max_brightness_nits: max_nits,
            min_brightness_nits: 0.1,
        }
    }

    /// Create Dolby Vision configuration.
    pub fn dolby_vision(max_nits: u32) -> Self {
        Self {
            internal_precision: BufferPrecision::FP32,
            output_colorspace: Colorspace::DolbyVision,
            max_brightness_nits: max_nits,
            min_brightness_nits: 0.1,
        }
    }

    /// Create linear HDR configuration (no tone mapping).
    pub fn linear_hdr() -> Self {
        Self {
            internal_precision: BufferPrecision::FP32,
            output_colorspace: Colorspace::Linear,
            max_brightness_nits: 10_000,
            min_brightness_nits: 0.0,
        }
    }

    /// Set the internal buffer precision.
    pub fn with_precision(mut self, precision: BufferPrecision) -> Self {
        self.internal_precision = precision;
        self
    }

    /// Set the output colorspace.
    pub fn with_colorspace(mut self, colorspace: Colorspace) -> Self {
        self.output_colorspace = colorspace;
        self
    }

    /// Set the maximum brightness in nits.
    pub fn with_max_brightness(mut self, max_nits: u32) -> Self {
        self.max_brightness_nits = max_nits;
        self
    }

    /// Set the minimum brightness in nits.
    pub fn with_min_brightness(mut self, min_nits: f64) -> Self {
        self.min_brightness_nits = min_nits;
        self
    }

    /// Returns true if this configuration uses HDR.
    pub fn is_hdr(&self) -> bool {
        self.output_colorspace.is_hdr()
    }

    /// Returns the wgpu texture format for color buffers with this precision.
    pub fn rgba_format(&self) -> wgpu::TextureFormat {
        self.internal_precision.rgba_format()
    }

    /// Returns the wgpu texture format for RG color buffers.
    pub fn rg_format(&self) -> wgpu::TextureFormat {
        self.internal_precision.rg_format()
    }

    /// Returns the wgpu texture format for R color buffers.
    pub fn r_format(&self) -> wgpu::TextureFormat {
        self.internal_precision.r_format()
    }

    /// Returns the wgpu texture format for depth buffers (always FP32).
    pub fn depth_format(&self) -> wgpu::TextureFormat {
        wgpu::TextureFormat::Depth32Float
    }

    /// Returns the final output format based on colorspace.
    pub fn output_format(&self) -> wgpu::TextureFormat {
        self.output_colorspace.output_format()
    }

    /// Calculate PQ encoding value for HDR10 (ST2084).
    pub fn encode_pq(luminance: f64, max_nits: u32) -> f64 {
        let c1 = 0.27928;
        let c2 = 0.25836;
        let c3 = 1.7883;

        let s = (luminance / max_nits as f64).sqrt();
        ((c1 + c2 * s) / (1.0 + c3 * s)).powf(1.0 / 24.0)
    }

    /// Calculate PQ decoding value for HDR10 (ST2084).
    pub fn decode_pq(value: f64, max_nits: u32) -> f64 {
        let c1 = 0.27928;
        let c2 = 0.25836;
        let c3 = 1.7883;

        let t = value.powf(24.0);
        let s = ((t - c1) / (c2 - c3 * t)).max(0.0).sqrt();
        s * max_nits as f64
    }
}
