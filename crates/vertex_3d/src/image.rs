//! Unified image resource types used for sampled textures, render targets, and storage images.

/// High-level image allocation description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageDesc {
    pub size: [u32; 3],
    pub format: wgpu::TextureFormat,
    pub mip_levels: u32,
    pub samples: u32,
    pub dimension: wgpu::TextureDimension,
    pub usage: wgpu::TextureUsages,
}

impl ImageDesc {
    pub fn new_2d(width: u32, height: u32, format: wgpu::TextureFormat) -> Self {
        Self {
            size: [width.max(1), height.max(1), 1],
            format,
            mip_levels: 1,
            samples: 1,
            dimension: wgpu::TextureDimension::D2,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        }
    }

    pub fn with_depth(mut self, depth_or_layers: u32) -> Self {
        self.size[2] = depth_or_layers.max(1);
        self
    }

    pub fn with_mip_levels(mut self, mip_levels: u32) -> Self {
        self.mip_levels = mip_levels.max(1);
        self
    }

    pub fn with_samples(mut self, samples: u32) -> Self {
        self.samples = samples.max(1);
        self
    }

    pub fn with_usage(mut self, usage: wgpu::TextureUsages) -> Self {
        self.usage = usage;
        self
    }

    pub fn with_dimension(mut self, dimension: wgpu::TextureDimension) -> Self {
        self.dimension = dimension;
        self
    }

    pub fn extent(&self) -> wgpu::Extent3d {
        wgpu::Extent3d {
            width: self.size[0],
            height: self.size[1],
            depth_or_array_layers: self.size[2],
        }
    }
}

/// CPU-side image asset metadata and intended GPU usage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageAsset {
    pub label: String,
    pub source_path: Option<String>,
    pub desc: ImageDesc,
}

impl ImageAsset {
    pub fn new(
        label: impl Into<String>,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> Self {
        Self {
            label: label.into(),
            source_path: None,
            desc: ImageDesc::new_2d(width, height, format),
        }
    }

    pub fn with_source_path(mut self, source_path: impl Into<String>) -> Self {
        self.source_path = Some(source_path.into());
        self
    }

    pub fn with_desc(mut self, desc: ImageDesc) -> Self {
        self.desc = desc;
        self
    }
}

/// Lightweight usage view over an image allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageViewDesc {
    pub base_mip_level: u32,
    pub mip_level_count: Option<u32>,
    pub base_array_layer: u32,
    pub array_layer_count: Option<u32>,
    pub aspect: wgpu::TextureAspect,
}

impl Default for ImageViewDesc {
    fn default() -> Self {
        Self {
            base_mip_level: 0,
            mip_level_count: None,
            base_array_layer: 0,
            array_layer_count: None,
            aspect: wgpu::TextureAspect::All,
        }
    }
}
