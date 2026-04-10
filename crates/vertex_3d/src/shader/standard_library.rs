//! Standard Slang modules for shared scene, global, and material interfaces.

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandardShaderImport {
    Globals,
    Material,
    Scene,
}

impl StandardShaderImport {
    pub fn module_name(self) -> &'static str {
        match self {
            Self::Globals => "vertex3d_globals",
            Self::Material => "vertex3d_material",
            Self::Scene => "vertex3d_scene",
        }
    }

    pub fn import_statement(self) -> &'static str {
        match self {
            Self::Globals => "import vertex3d_globals;",
            Self::Material => "import vertex3d_material;",
            Self::Scene => "import vertex3d_scene;",
        }
    }

    pub fn file_name(self) -> &'static str {
        match self {
            Self::Globals => "vertex3d_globals.slang",
            Self::Material => "vertex3d_material.slang",
            Self::Scene => "vertex3d_scene.slang",
        }
    }
}

pub fn standard_library_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

pub fn standard_module_path(import: StandardShaderImport) -> PathBuf {
    standard_library_dir().join(import.file_name())
}

pub fn resolve_standard_import_path(module_name: &str) -> Option<PathBuf> {
    let import = match module_name {
        "vertex3d_globals" => StandardShaderImport::Globals,
        "vertex3d_material" => StandardShaderImport::Material,
        "vertex3d_scene" => StandardShaderImport::Scene,
        _ => return None,
    };
    Some(standard_module_path(import))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_standard_module_paths() {
        assert!(
            resolve_standard_import_path("vertex3d_scene")
                .unwrap()
                .ends_with("vertex3d_scene.slang")
        );
    }
}
