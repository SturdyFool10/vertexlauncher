fn main() {
    #[cfg(target_os = "windows")]
    {
        println!("cargo:rerun-if-changed=../launcher_ui/src/assets/vertex.webp");
        if let Err(error) = compile_windows_resources() {
            println!("cargo:warning=failed to configure Windows resources: {error}");
        }
    }
}

#[cfg(target_os = "windows")]
fn compile_windows_resources() -> Result<(), String> {
    use image::{
        ExtendedColorType, ImageEncoder, ImageReader, codecs::ico::IcoEncoder, imageops::FilterType,
    };
    use std::{fs::File, io::Cursor, path::PathBuf};

    let out_dir = std::env::var("OUT_DIR")
        .map(PathBuf::from)
        .map_err(|error| format!("OUT_DIR is not set: {error}"))?;
    let icon_path = out_dir.join("vertex.ico");
    let decoded = ImageReader::new(Cursor::new(include_bytes!(
        "../launcher_ui/src/assets/vertex.webp"
    )))
    .with_guessed_format()
    .map_err(|error| format!("failed to detect vertex icon format: {error}"))?
    .decode()
    .map_err(|error| format!("failed to decode vertex icon source image: {error}"))?;
    let resized = decoded.resize(256, 256, FilterType::Lanczos3).to_rgba8();
    let mut icon_file = File::create(&icon_path)
        .map_err(|error| format!("failed to create generated .ico icon: {error}"))?;
    IcoEncoder::new(&mut icon_file)
        .write_image(
            resized.as_raw(),
            resized.width(),
            resized.height(),
            ExtendedColorType::Rgba8,
        )
        .map_err(|error| format!("failed to write generated .ico icon: {error}"))?;

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon(icon_path.to_string_lossy().as_ref());
    resource
        .compile()
        .map_err(|error| format!("failed to compile Windows resources: {error}"))?;
    Ok(())
}
