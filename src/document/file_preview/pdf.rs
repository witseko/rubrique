//! Rasterize one PDF page at a time with a bounded output size.

use std::sync::Arc;

use eframe::egui::ColorImage;
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings};

use super::Content;

/// Maximum pixel size of a rendered page along its longest side.
const LONG_SIDE: f32 = 1800.0;

/// Render the selected page to an opaque bitmap on a white background.
pub(super) fn page(bytes: Arc<Vec<u8>>, selected: usize) -> Result<Content, String> {
    let pdf = Pdf::new(bytes).map_err(|error| format!("Cannot read this PDF: {error:?}"))?;
    let pages = pdf.pages();
    let page = pages.get(selected).ok_or("This PDF has no page to display.")?;
    let (width, height) = page.render_dimensions();
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err("This PDF page has invalid dimensions.".into());
    }
    let scale = LONG_SIDE / width.max(height);
    if !scale.is_finite() || width * scale < 1.0 || height * scale < 1.0 {
        return Err("This PDF page has invalid proportions.".into());
    }
    let bitmap = hayro::render(
        page,
        &RenderCache::new(),
        &InterpreterSettings::default(),
        &RenderSettings::default(),
        &PixmapSettings { x_scale: scale, y_scale: scale, bg_color: WHITE },
    );
    let size = [usize::from(bitmap.width()), usize::from(bitmap.height())];
    let pixels = ColorImage::from_rgba_premultiplied(size, bitmap.data_as_u8_slice());
    Ok(Content::Page { pixels: Some(pixels), page_count: pages.len() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Color32;

    #[test]
    fn renders_the_requested_pdf_page() {
        let bytes = Arc::new(include_bytes!("fixtures/two-pages.pdf").to_vec());
        for (index, color) in [Color32::RED, Color32::BLUE].into_iter().enumerate() {
            let Content::Page { pixels: Some(image), page_count } = page(Arc::clone(&bytes), index).unwrap() else {
                panic!("expected pixels for the PDF page");
            };
            assert_eq!(page_count, 2);
            assert_eq!(image.pixels[image.pixels.len() / 2], color);
        }
        assert!(page(bytes, 2).is_err());
    }

    #[test]
    fn rejects_an_invalid_pdf() {
        assert!(page(Arc::new(b"invalid PDF".to_vec()), 0).is_err());
    }
}
