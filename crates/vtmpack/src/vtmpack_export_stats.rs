#[derive(Debug, Clone, Copy)]
pub struct VtmpackExportStats {
    /// Mod files embedded as raw bytes (Modrinth lookup failed or not attempted).
    pub bundled_mod_files: usize,
    /// Mod files resolved to a Modrinth version and stored as manifest entries
    /// rather than embedded bytes, keeping the archive small.
    pub downloadable_mod_files: usize,
    pub config_files: usize,
    pub additional_files: usize,
}

#[derive(Debug, Clone)]
pub struct VtmpackExportProgress {
    pub message: String,
    pub completed_steps: usize,
    pub total_steps: usize,
}
