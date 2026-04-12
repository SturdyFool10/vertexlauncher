use super::*;

pub struct TextUi {
    pub(crate) font_system: FontSystem,
    pub(crate) scale_context: ScaleContext,
    pub(crate) syntax_set: SyntaxSet,
    pub(crate) code_theme: Theme,
    pub(crate) prepared_texts: ThreadSafeLru<Id, PreparedTextCacheEntry>,
    pub(crate) glyph_atlas: GlyphAtlas,
    pub(crate) input_states: FxHashMap<Id, InputState>,
    pub(crate) ui_font_family: Option<String>,
    pub(crate) ui_font_size_scale: f32,
    pub(crate) ui_font_weight: i32,
    pub(crate) open_type_features_enabled: bool,
    pub(crate) open_type_features_to_enable: String,
    pub(crate) open_type_feature_tags: Vec<[u8; 4]>,
    pub(crate) open_type_features: Option<FontFeatures>,
    pub(crate) async_raster: AsyncRasterState,
    pub(crate) graphics_config: TextGraphicsConfig,
    pub(crate) current_frame: u64,
    pub(crate) max_texture_side_px: usize,
    pub(crate) frame_events: Vec<TextInputEvent>,
    pub(crate) markdown_cache: FxHashMap<Id, (u64, u64, Arc<[TextMarkdownBlock]>)>,
    pub(crate) gpu_scene_cache: ThreadSafeLru<u64, Arc<TextGpuScene>>,
    pub(crate) gpu_scene_page_batch_cache: ThreadSafeLru<u64, Arc<[TextGpuScenePageBatch]>>,
    pub(crate) gpu_scene_draw_batch_cache: ThreadSafeLru<u64, Arc<[TextGpuScenePageBatch]>>,
    pub(crate) gpu_scene_glyph_cache: ThreadSafeLru<GlyphRasterKey, Arc<PreparedAtlasGlyph>>,
    /// Reusable CPU-side atlas pages to avoid per-frame alloc/free of large pixel buffers.
    pub(crate) cpu_page_pool: Vec<CpuSceneAtlasPage>,
}

impl Default for TextUi {
    fn default() -> Self {
        Self::new()
    }
}

impl TextUi {
    pub fn new() -> Self {
        Self::new_with_graphics_config(TextGraphicsConfig::default())
    }

    pub fn new_with_graphics_config(graphics_config: TextGraphicsConfig) -> Self {
        let syntax_set = SyntaxSet::load_defaults_newlines();
        let theme_set = ThemeSet::load_defaults();
        let code_theme = theme_set
            .themes
            .get("base16-ocean.dark")
            .or_else(|| theme_set.themes.values().next())
            .cloned()
            .unwrap_or_else(|| {
                warn!(
                    target: "vertexlauncher/textui",
                    "syntect theme set was unexpectedly empty; using default code theme"
                );
                Theme::default()
            });

        let glyph_atlas = GlyphAtlas::new();
        let mut font_system = FontSystem::new();
        configure_text_font_defaults(&mut font_system);
        Self {
            font_system,
            scale_context: ScaleContext::new(),
            syntax_set,
            code_theme,
            prepared_texts: ThreadSafeLru::new(PREPARED_TEXT_CACHE_MAX_BYTES),
            glyph_atlas,
            input_states: FxHashMap::default(),
            ui_font_family: None,
            ui_font_size_scale: 1.0,
            ui_font_weight: 400,
            open_type_features_enabled: false,
            open_type_features_to_enable: String::new(),
            open_type_feature_tags: Vec::new(),
            open_type_features: None,
            async_raster: new_async_raster_state(),
            graphics_config,
            current_frame: 0,
            max_texture_side_px: usize::MAX,
            frame_events: Vec::new(),
            markdown_cache: FxHashMap::default(),
            gpu_scene_cache: ThreadSafeLru::new(GPU_SCENE_CACHE_MAX_BYTES),
            gpu_scene_page_batch_cache: ThreadSafeLru::new(GPU_SCENE_PAGE_BATCH_CACHE_MAX_BYTES),
            gpu_scene_draw_batch_cache: ThreadSafeLru::new(GPU_SCENE_DRAW_BATCH_CACHE_MAX_BYTES),
            gpu_scene_glyph_cache: ThreadSafeLru::new(GPU_SCENE_GLYPH_CACHE_MAX_BYTES),
            cpu_page_pool: Vec::new(),
        }
    }
}
