use anyhow::{anyhow, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};

const EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "avif", "bmp"];

#[derive(Debug, Clone, Serialize)]
pub struct WallpaperEntry {
    pub name: String,
    pub path: String,
    pub width: u32,
    pub height: u32,
}

pub fn default_directory() -> PathBuf {
    if let Ok(path) = std::env::var("WYRD_WALLPAPER_DIR") {
        return PathBuf::from(path);
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Pictures/Wallpapers")
}

pub fn list(directory: &Path) -> Vec<WallpaperEntry> {
    let mut entries = std::fs::read_dir(directory)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            let is_image = path.is_file()
                && path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
                    .unwrap_or(false);
            if !is_image {
                return None;
            }
            let (width, height) = image::image_dimensions(&path).ok()?;
            Some(WallpaperEntry {
                name: path.file_name()?.to_string_lossy().to_string(),
                path: path.to_string_lossy().to_string(),
                width,
                height,
            })
        })
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.name.to_ascii_lowercase());
    entries
}

pub fn resolve(input: &Path, directory: &Path) -> Result<PathBuf> {
    let path = if input.is_absolute() {
        input.to_path_buf()
    } else {
        directory.join(input)
    };
    let path = path
        .canonicalize()
        .map_err(|_| anyhow!("wallpaper does not exist: {}", path.display()))?;
    if !path.is_file() {
        return Err(anyhow!("wallpaper is not a file: {}", path.display()));
    }
    if image::image_dimensions(&path).is_err() {
        return Err(anyhow!(
            "unsupported or unreadable image: {}",
            path.display()
        ));
    }
    Ok(path)
}

use material_colors::{
    image::{FilterType, ImageReader},
    theme::ThemeBuilder,
};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct MaterialScheme {
    pub primary: String,
    pub on_primary: String,
    pub primary_container: String,
    pub on_primary_container: String,
    pub secondary: String,
    pub on_secondary: String,
    pub tertiary: String,
    pub on_tertiary: String,
    pub background: String,
    pub on_background: String,
    pub surface: String,
    pub on_surface: String,
    pub surface_container: String,
    pub surface_variant: String,
    pub on_surface_variant: String,
    pub outline: String,
    pub outline_variant: String,
}

impl MaterialScheme {
    pub fn to_map(&self) -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("primary".to_string(), self.primary.clone());
        m.insert("on_primary".to_string(), self.on_primary.clone());
        m.insert(
            "primary_container".to_string(),
            self.primary_container.clone(),
        );
        m.insert(
            "on_primary_container".to_string(),
            self.on_primary_container.clone(),
        );
        m.insert("secondary".to_string(), self.secondary.clone());
        m.insert("on_secondary".to_string(), self.on_secondary.clone());
        m.insert("tertiary".to_string(), self.tertiary.clone());
        m.insert("on_tertiary".to_string(), self.on_tertiary.clone());
        m.insert("background".to_string(), self.background.clone());
        m.insert("on_background".to_string(), self.on_background.clone());
        m.insert("surface".to_string(), self.surface.clone());
        m.insert("on_surface".to_string(), self.on_surface.clone());
        m.insert(
            "surface_container".to_string(),
            self.surface_container.clone(),
        );
        m.insert("surface_variant".to_string(), self.surface_variant.clone());
        m.insert(
            "on_surface_variant".to_string(),
            self.on_surface_variant.clone(),
        );
        m.insert("outline".to_string(), self.outline.clone());
        m.insert("outline_variant".to_string(), self.outline_variant.clone());
        m.insert("accent".to_string(), self.primary.clone());
        m
    }
}

#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct MaterialPalette {
    pub dark: MaterialScheme,
    pub light: MaterialScheme,
}

pub fn fallback_palette() -> MaterialPalette {
    let dark = MaterialScheme {
        primary: "#7dd3fc".to_string(),
        on_primary: "#003548".to_string(),
        primary_container: "#004d67".to_string(),
        on_primary_container: "#bfe9ff".to_string(),
        secondary: "#b3cad5".to_string(),
        on_secondary: "#1e333c".to_string(),
        tertiary: "#c6c2ea".to_string(),
        on_tertiary: "#2e2d4d".to_string(),
        background: "#0c0e16".to_string(),
        on_background: "#e2e8f0".to_string(),
        surface: "#121722".to_string(),
        on_surface: "#e2e8f0".to_string(),
        surface_container: "#182030".to_string(),
        surface_variant: "#2a3447".to_string(),
        on_surface_variant: "#94a3b8".to_string(),
        outline: "#38455e".to_string(),
        outline_variant: "#242d3d".to_string(),
    };
    let light = MaterialScheme {
        primary: "#0284c7".to_string(),
        on_primary: "#ffffff".to_string(),
        primary_container: "#bfe9ff".to_string(),
        on_primary_container: "#001f2a".to_string(),
        secondary: "#4b626d".to_string(),
        on_secondary: "#ffffff".to_string(),
        tertiary: "#5d5b7e".to_string(),
        on_tertiary: "#ffffff".to_string(),
        background: "#f8fafc".to_string(),
        on_background: "#0f172a".to_string(),
        surface: "#ffffff".to_string(),
        on_surface: "#0f172a".to_string(),
        surface_container: "#f1f5f9".to_string(),
        surface_variant: "#e2e8f0".to_string(),
        on_surface_variant: "#64748b".to_string(),
        outline: "#cbd5e1".to_string(),
        outline_variant: "#e2e8f0".to_string(),
    };
    MaterialPalette { dark, light }
}

fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let rf = r as f32 / 255.0;
    let gf = g as f32 / 255.0;
    let bf = b as f32 / 255.0;
    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let l = (max + min) * 0.5;
    let d = max - min;
    if d.abs() < 1e-5 {
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let mut h = if (max - rf).abs() < 1e-5 {
        (gf - bf) / d + if gf < bf { 6.0 } else { 0.0 }
    } else if (max - gf).abs() < 1e-5 {
        (bf - rf) / d + 2.0
    } else {
        (rf - gf) / d + 4.0
    };
    h /= 6.0;
    (h * 360.0, s, l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let s = s.clamp(0.0, 1.0);
    let l = l.clamp(0.0, 1.0);
    if s < 1e-5 {
        let v = (l * 255.0).round() as u8;
        return (v, v, v);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let hk = (h.rem_euclid(360.0)) / 360.0;
    let hue_to_rgb = |mut t: f32| -> f32 {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            return p + (q - p) * 6.0 * t;
        }
        if t < 0.5 {
            return q;
        }
        if t < 2.0 / 3.0 {
            return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
        }
        p
    };
    let r = (hue_to_rgb(hk + 1.0 / 3.0) * 255.0).round() as u8;
    let g = (hue_to_rgb(hk) * 255.0).round() as u8;
    let b = (hue_to_rgb(hk - 1.0 / 3.0) * 255.0).round() as u8;
    (r, g, b)
}

fn adjust_hex_hsl(hex: &str, min_l: f32, max_l: f32, min_s: f32, max_s: f32) -> String {
    let clean = hex.trim_start_matches('#');
    if clean.len() != 6 || !clean.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return "#808080".to_string();
    }
    let (Ok(r), Ok(g), Ok(b)) = (
        u8::from_str_radix(&clean[0..2], 16),
        u8::from_str_radix(&clean[2..4], 16),
        u8::from_str_radix(&clean[4..6], 16),
    ) else {
        return "#808080".to_string();
    };
    let (h, s, l) = rgb_to_hsl(r, g, b);
    let new_s = if s > 0.04 {
        s.clamp(min_s, max_s)
    } else {
        s.min(max_s)
    };
    let new_l = l.clamp(min_l, max_l);
    let (nr, ng, nb) = hsl_to_rgb(h, new_s, new_l);
    format!("#{:02x}{:02x}{:02x}", nr, ng, nb)
}

fn scheme_from_m3_dark(s: &material_colors::scheme::Scheme) -> MaterialScheme {
    MaterialScheme {
        primary: adjust_hex_hsl(&s.primary.to_hex_with_pound(), 0.68, 0.78, 0.62, 0.95),
        on_primary: adjust_hex_hsl(&s.on_primary.to_hex_with_pound(), 0.08, 0.16, 0.30, 0.80),
        primary_container: adjust_hex_hsl(
            &s.primary_container.to_hex_with_pound(),
            0.20,
            0.28,
            0.40,
            0.85,
        ),
        on_primary_container: adjust_hex_hsl(
            &s.on_primary_container.to_hex_with_pound(),
            0.86,
            0.94,
            0.40,
            0.95,
        ),
        secondary: adjust_hex_hsl(&s.secondary.to_hex_with_pound(), 0.72, 0.80, 0.38, 0.85),
        on_secondary: adjust_hex_hsl(&s.on_secondary.to_hex_with_pound(), 0.10, 0.18, 0.25, 0.75),
        tertiary: adjust_hex_hsl(&s.tertiary.to_hex_with_pound(), 0.72, 0.82, 0.45, 0.90),
        on_tertiary: adjust_hex_hsl(&s.on_tertiary.to_hex_with_pound(), 0.10, 0.18, 0.25, 0.75),
        background: adjust_hex_hsl(&s.background.to_hex_with_pound(), 0.05, 0.08, 0.15, 0.45),
        on_background: adjust_hex_hsl(&s.on_background.to_hex_with_pound(), 0.92, 0.97, 0.10, 0.35),
        surface: adjust_hex_hsl(&s.surface.to_hex_with_pound(), 0.07, 0.10, 0.15, 0.45),
        on_surface: adjust_hex_hsl(&s.on_surface.to_hex_with_pound(), 0.93, 0.97, 0.10, 0.35),
        surface_container: adjust_hex_hsl(
            &s.surface_container.to_hex_with_pound(),
            0.10,
            0.14,
            0.18,
            0.48,
        ),
        surface_variant: adjust_hex_hsl(
            &s.surface_variant.to_hex_with_pound(),
            0.16,
            0.22,
            0.18,
            0.48,
        ),
        on_surface_variant: adjust_hex_hsl(
            &s.on_surface_variant.to_hex_with_pound(),
            0.78,
            0.85,
            0.12,
            0.40,
        ),
        outline: adjust_hex_hsl(&s.outline.to_hex_with_pound(), 0.42, 0.52, 0.12, 0.40),
        outline_variant: adjust_hex_hsl(
            &s.outline_variant.to_hex_with_pound(),
            0.22,
            0.28,
            0.12,
            0.40,
        ),
    }
}

fn scheme_from_m3(s: &material_colors::scheme::Scheme) -> MaterialScheme {
    MaterialScheme {
        primary: s.primary.to_hex_with_pound(),
        on_primary: s.on_primary.to_hex_with_pound(),
        primary_container: s.primary_container.to_hex_with_pound(),
        on_primary_container: s.on_primary_container.to_hex_with_pound(),
        secondary: s.secondary.to_hex_with_pound(),
        on_secondary: s.on_secondary.to_hex_with_pound(),
        tertiary: s.tertiary.to_hex_with_pound(),
        on_tertiary: s.on_tertiary.to_hex_with_pound(),
        background: s.background.to_hex_with_pound(),
        on_background: s.on_background.to_hex_with_pound(),
        surface: s.surface.to_hex_with_pound(),
        on_surface: s.on_surface.to_hex_with_pound(),
        surface_container: s.surface_container.to_hex_with_pound(),
        surface_variant: s.surface_variant.to_hex_with_pound(),
        on_surface_variant: s.on_surface_variant.to_hex_with_pound(),
        outline: s.outline.to_hex_with_pound(),
        outline_variant: s.outline_variant.to_hex_with_pound(),
    }
}

pub fn palette_from_source(source_color: material_colors::color::Argb) -> MaterialPalette {
    let theme = ThemeBuilder::with_source(source_color).build();
    MaterialPalette {
        dark: scheme_from_m3_dark(&theme.schemes.dark),
        light: scheme_from_m3(&theme.schemes.light),
    }
}

pub fn palette_from_hex(hex: &str) -> Option<MaterialPalette> {
    use std::str::FromStr;
    material_colors::color::Argb::from_str(hex)
        .ok()
        .map(palette_from_source)
}

fn extract_vibrant_argb(path: &Path) -> Option<material_colors::color::Argb> {
    let img = image::open(path).ok()?;
    let small = img
        .resize_exact(96, 96, image::imageops::FilterType::Triangle)
        .to_rgba8();
    let mut bins_weight = [0.0f32; 36];
    let mut bins_r = [0.0f32; 36];
    let mut bins_g = [0.0f32; 36];
    let mut bins_b = [0.0f32; 36];

    for pixel in small.pixels() {
        let [r, g, b, a] = pixel.0;
        if a < 128 {
            continue;
        }
        let (h, s, l) = rgb_to_hsl(r, g, b);
        if s < 0.18 || !(0.10..=0.88).contains(&l) {
            continue;
        }
        let mid_l_penalty = 1.0 - ((l - 0.52).abs() * 1.35).clamp(0.0, 0.85);
        let weight = s.powf(1.6) * mid_l_penalty;
        let bin = ((h / 10.0).floor() as usize).min(35);
        bins_weight[bin] += weight;
        bins_r[bin] += r as f32 * weight;
        bins_g[bin] += g as f32 * weight;
        bins_b[bin] += b as f32 * weight;
    }

    let (best_bin, &best_w) = bins_weight
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))?;

    if best_w < 1.0 {
        return None;
    }

    let avg_r = (bins_r[best_bin] / best_w).round() as u8;
    let avg_g = (bins_g[best_bin] / best_w).round() as u8;
    let avg_b = (bins_b[best_bin] / best_w).round() as u8;
    let (h, s, l) = rgb_to_hsl(avg_r, avg_g, avg_b);
    let (vr, vg, vb) = hsl_to_rgb(h, s.clamp(0.60, 0.92), l.clamp(0.48, 0.62));
    Some(material_colors::color::Argb::new(255, vr, vg, vb))
}

pub fn extract_palette(path: &Path) -> MaterialPalette {
    if let Some(vibrant_color) = extract_vibrant_argb(path) {
        return palette_from_source(vibrant_color);
    }
    if let Ok(bytes) = std::fs::read(path) {
        if let Ok(mut data) = ImageReader::read(bytes) {
            data.resize(128, 128, FilterType::Lanczos3);
            let source_color = ImageReader::extract_color(&data);
            return palette_from_source(source_color);
        }
    }
    fallback_palette()
}

#[allow(dead_code)]
pub fn accent(path: &Path) -> String {
    extract_palette(path).dark.primary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_paths_are_rejected() {
        let dir = std::env::temp_dir().join(format!("wyrd-wallpaper-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("not-an-image.txt");
        std::fs::write(&path, "nope").unwrap();
        assert!(resolve(Path::new("not-an-image.txt"), &dir).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn accent_extraction_picks_vibrant_color() {
        let dir = std::env::temp_dir().join(format!("wyrd-accent-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.png");

        // Create an image: 50% dark background, 50% vibrant sky blue (#7dd3fc)
        let mut img = image::RgbImage::new(16, 16);
        for (x, _y, pixel) in img.enumerate_pixels_mut() {
            if x < 8 {
                *pixel = image::Rgb([125, 211, 252]); // Vibrant sky blue
            } else {
                *pixel = image::Rgb([20, 20, 25]); // Dark background
            }
        }
        img.save(&path).unwrap();

        let extracted = accent(&path);
        assert!(extracted.starts_with('#'));
        assert_eq!(extracted.len(), 7);
        // The extracted color should prioritize the vibrant sky-blue over the dark background
        let _r = u8::from_str_radix(&extracted[1..3], 16).unwrap();
        let g = u8::from_str_radix(&extracted[3..5], 16).unwrap();
        let b = u8::from_str_radix(&extracted[5..7], 16).unwrap();
        assert!(
            g > 150,
            "Expected vibrant green component in blue accent, got {}",
            g
        );
        assert!(b > 180, "Expected vibrant blue component, got {}", b);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_adjust_hex_hsl_clamping_and_format() {
        let adjusted = adjust_hex_hsl("#336699", 0.25, 0.85, 0.35, 0.70);
        assert_eq!(adjusted.len(), 7);
        assert!(adjusted.starts_with('#'));
        assert!(adjusted[1..].chars().all(|c| c.is_ascii_hexdigit()));

        // Very bright color clamped down to dark surface range [0.05, 0.12]
        let clamped_dark = adjust_hex_hsl("#f5f5ff", 0.05, 0.12, 0.10, 0.35);
        let r = u8::from_str_radix(&clamped_dark[1..3], 16).unwrap();
        let g = u8::from_str_radix(&clamped_dark[3..5], 16).unwrap();
        let b = u8::from_str_radix(&clamped_dark[5..7], 16).unwrap();
        assert!(
            r < 50 && g < 50 && b < 60,
            "Expected dark clamped RGB, got {clamped_dark}"
        );

        // Invalid hex input is explicitly replaced with neutral grey.
        let fallback = adjust_hex_hsl("invalid", 0.25, 0.85, 0.0, 1.0);
        assert_eq!(fallback, "#808080");
        assert_eq!(adjust_hex_hsl("#12xz56", 0.25, 0.85, 0.0, 1.0), "#808080");
        assert_eq!(adjust_hex_hsl("#123", 0.25, 0.85, 0.0, 1.0), "#808080");
    }
}
