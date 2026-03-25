use config::SvgAaMode;
use std::sync::atomic::{AtomicU8, Ordering};

static SVG_AA_MODE: AtomicU8 = AtomicU8::new(SvgAaMode::Balanced as u8);

pub fn set_svg_aa_mode(mode: SvgAaMode) {
    SVG_AA_MODE.store(mode as u8, Ordering::Relaxed);
}

pub fn get_svg_aa_mode() -> SvgAaMode {
    match SVG_AA_MODE.load(Ordering::Relaxed) {
        x if x == SvgAaMode::Off as u8 => SvgAaMode::Off,
        x if x == SvgAaMode::Balanced as u8 => SvgAaMode::Balanced,
        x if x == SvgAaMode::Crisp as u8 => SvgAaMode::Crisp,
        x if x == SvgAaMode::Ultra as u8 => SvgAaMode::Ultra,
        _ => SvgAaMode::Balanced,
    }
}

pub fn supersample_scale() -> u32 {
    get_svg_aa_mode().supersample_scale().max(1)
}
