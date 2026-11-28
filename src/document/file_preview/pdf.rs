//! Rasterize one PDF page at a time with a bounded output size, using the
//! system's PDF renderer: Core Graphics on macOS, poppler's `pdftoppm` on
//! Linux.

use std::sync::Arc;

use super::Content;

/// Maximum pixel size of a rendered page along its longest side.
const LONG_SIDE: f64 = 1800.0;

/// Render the selected page to an opaque bitmap on a white background.
pub(super) fn page(bytes: &Arc<Vec<u8>>, selected: usize) -> Result<Content, String> {
    let (pixels, page_count) = platform::render(bytes, selected)?;
    Ok(Content::Page { pixels: Some(pixels), page_count })
}

#[cfg(target_os = "macos")]
mod platform {
    use eframe::egui::ColorImage;
    use objc2_core_foundation::{CFData, CGPoint, CGRect, CGSize};
    use objc2_core_graphics::{
        CGBitmapContextCreate, CGBitmapContextCreateImage, CGColorSpace, CGContext, CGDataProvider, CGImage,
        CGImageAlphaInfo, CGImageByteOrderInfo, CGPDFBox, CGPDFDocument, CGPDFPage,
    };

    use super::LONG_SIDE;

    /// The bitmap size for a page of `width` by `height` points, scaled so the
    /// longer side is `LONG_SIDE` pixels.
    pub(super) fn pixel_size(width: f64, height: f64) -> Result<[usize; 2], String> {
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            return Err("This PDF page has invalid dimensions.".into());
        }
        let scale = LONG_SIDE / width.max(height);
        let (pixel_width, pixel_height) = ((width * scale).round(), (height * scale).round());
        if pixel_width < 1.0 || pixel_height < 1.0 {
            return Err("This PDF page has invalid proportions.".into());
        }
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "both sides are rounded and lie between 1 and LONG_SIDE"
        )]
        Ok([pixel_width as usize, pixel_height as usize])
    }

    fn unreadable() -> String {
        "Cannot read this PDF.".into()
    }

    /// Draw the page into a bitmap context and copy out its pixels.
    pub(super) fn render(bytes: &[u8], selected: usize) -> Result<(ColorImage, usize), String> {
        let data = CFData::from_bytes(bytes);
        let provider = CGDataProvider::with_cf_data(Some(&data)).ok_or_else(unreadable)?;
        let document = CGPDFDocument::with_provider(Some(&provider)).ok_or_else(unreadable)?;
        if !CGPDFDocument::is_unlocked(Some(&document)) {
            return Err("This PDF is password protected.".into());
        }
        let page_count = CGPDFDocument::number_of_pages(Some(&document));
        // Core Graphics numbers pages from 1.
        let page = CGPDFDocument::page(Some(&document), selected + 1).ok_or("This PDF has no page to display.")?;
        let crop = CGPDFPage::box_rect(Some(&page), CGPDFBox::CropBox);
        let sideways = CGPDFPage::rotation_angle(Some(&page)).rem_euclid(180) == 90;
        let (width, height) =
            if sideways { (crop.size.height, crop.size.width) } else { (crop.size.width, crop.size.height) };
        let [pixel_width, pixel_height] = pixel_size(width, height)?;

        let space = CGColorSpace::new_device_rgb();
        let context = bitmap_context(pixel_width, pixel_height, space.as_deref()).ok_or_else(unreadable)?;
        let context = Some(&*context);
        let scale = LONG_SIDE / width.max(height);
        CGContext::set_rgb_fill_color(context, 1.0, 1.0, 1.0, 1.0);
        CGContext::fill_rect(context, CGRect::new(CGPoint::ZERO, CGSize::new(width * scale, height * scale)));
        CGContext::scale_ctm(context, scale, scale);
        // At the page's own size this transform only rotates and moves the
        // crop box into place (it never enlarges, hence the scale above).
        let page_rect = CGRect::new(CGPoint::ZERO, CGSize::new(width, height));
        CGContext::concat_ctm(
            context,
            CGPDFPage::drawing_transform(Some(&page), CGPDFBox::CropBox, page_rect, 0, true),
        );
        CGContext::draw_pdf_page(context, Some(&page));

        let image = CGBitmapContextCreateImage(context).ok_or_else(unreadable)?;
        let row_bytes = CGImage::bytes_per_row(Some(&image));
        let pixels = CGDataProvider::data(CGImage::data_provider(Some(&image)).as_deref()).ok_or_else(unreadable)?;
        let pixels = pixels.to_vec();
        let mut rgba = Vec::with_capacity(pixel_width * pixel_height * 4);
        for row in pixels.chunks(row_bytes.max(1)).take(pixel_height) {
            rgba.extend_from_slice(row.get(..pixel_width * 4).ok_or_else(unreadable)?);
        }
        if rgba.len() != pixel_width * pixel_height * 4 {
            return Err(unreadable());
        }
        Ok((ColorImage::from_rgba_premultiplied([pixel_width, pixel_height], &rgba), page_count))
    }

    /// An RGBA bitmap context whose memory Core Graphics allocates.
    #[expect(unsafe_code, reason = "the one Core Graphics call that takes a raw buffer pointer")]
    fn bitmap_context(
        width: usize,
        height: usize,
        space: Option<&CGColorSpace>,
    ) -> Option<objc2_core_foundation::CFRetained<CGContext>> {
        let info = CGImageAlphaInfo::PremultipliedLast.0 | CGImageByteOrderInfo::Order32Big.0;
        // SAFETY: a null buffer asks Core Graphics to allocate and own the
        // pixel memory; a bytes-per-row of 0 lets it pick the row length.
        unsafe { CGBitmapContextCreate(std::ptr::null_mut(), width, height, 8, 0, space, info) }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use std::io;
    use std::process::Command;
    use std::sync::Arc;

    use eframe::egui::ColorImage;

    use super::super::converter;

    /// Largest PNG read from `pdftoppm`.
    const MAX_PNG_BYTES: usize = 64 * 1024 * 1024;

    /// Count pages with `pdfinfo`, then render the page with `pdftoppm`.
    pub(super) fn render(bytes: &Arc<Vec<u8>>, selected: usize) -> Result<(ColorImage, usize), String> {
        let page_count = page_count(Arc::clone(bytes))?;
        if selected >= page_count {
            return Err("This PDF has no page to display.".into());
        }
        let number = (selected + 1).to_string();
        let long_side = super::LONG_SIDE.to_string();
        let mut command = Command::new("pdftoppm");
        // `-` reads the PDF from standard input; with no output name the PNG
        // goes to standard output.
        command.args(["-png", "-cropbox", "-f", &number, "-l", &number, "-scale-to", &long_side, "-"]);
        let png = converter::run(&mut command, Arc::clone(bytes), MAX_PNG_BYTES, "PDF renderer")
            .map_err(|error| missing_tool(&error))?;
        let image = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
            .map_err(|error| format!("Cannot read the rendered PDF page: {error}"))?
            .to_rgba8();
        let size = [image.width() as usize, image.height() as usize];
        Ok((ColorImage::from_rgba_unmultiplied(size, image.as_raw()), page_count))
    }

    /// The `Pages:` line of `pdfinfo`.
    fn page_count(bytes: Arc<Vec<u8>>) -> Result<usize, String> {
        let mut command = Command::new("pdfinfo");
        command.arg("-");
        let output =
            converter::run(&mut command, bytes, 64 * 1024, "PDF reader").map_err(|error| missing_tool(&error))?;
        String::from_utf8_lossy(&output)
            .lines()
            .find_map(|line| line.strip_prefix("Pages:"))
            .and_then(|count| count.trim().parse().ok())
            .ok_or_else(|| "Cannot read this PDF.".into())
    }

    /// Point to the package when the poppler tools are not installed.
    fn missing_tool(error: &io::Error) -> String {
        if error.kind() == io::ErrorKind::NotFound {
            "Install poppler-utils to preview PDF files.".into()
        } else {
            error.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Color32;

    /// Whether the system renderer is here (always on macOS; poppler-utils
    /// on Linux).
    fn renderer_available() -> bool {
        cfg!(target_os = "macos") || std::process::Command::new("pdftoppm").arg("-v").output().is_ok()
    }

    #[test]
    fn renders_the_requested_pdf_page() {
        if !renderer_available() {
            eprintln!("skipped: pdftoppm is not installed");
            return;
        }
        let bytes = Arc::new(include_bytes!("fixtures/two-pages.pdf").to_vec());
        for (index, color) in [Color32::RED, Color32::BLUE].into_iter().enumerate() {
            let Content::Page { pixels: Some(image), page_count } = page(&bytes, index).unwrap() else {
                panic!("expected pixels for the PDF page");
            };
            assert_eq!(page_count, 2);
            assert_eq!(image.size.iter().max(), Some(&1800));
            assert_eq!(image.pixels[image.pixels.len() / 2], color);
        }
        assert!(page(&bytes, 2).is_err());
    }

    #[test]
    fn rejects_an_invalid_pdf() {
        assert!(page(&Arc::new(b"invalid PDF".to_vec()), 0).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn scales_the_longer_side_to_the_limit() {
        use platform::pixel_size;
        assert_eq!(pixel_size(612.0, 792.0), Ok([1391, 1800]));
        assert!(pixel_size(0.0, 792.0).is_err());
        assert!(pixel_size(f64::NAN, 1.0).is_err());
        assert!(pixel_size(1.0, 1e9).is_err());
    }
}
