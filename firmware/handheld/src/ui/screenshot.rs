use std::{fs::File, io::Write, time::SystemTime};

use slint::platform::software_renderer::{LineBufferProvider, RepaintBufferType};

use crate::ui::slint::{Argb1555, MinimalSoftwareWindow};

struct ScreenshotLineBuffer<'a> {
    buffer: &'a mut [Argb1555],
    writer: &'a mut File,
    error: Option<std::io::Error>,
}

impl<'a> LineBufferProvider for &mut ScreenshotLineBuffer<'a> {
    type TargetPixel = Argb1555;

    fn process_line(
        &mut self,
        _line: usize,
        range: core::ops::Range<usize>,
        render_fn: impl FnOnce(&mut [Self::TargetPixel]),
    ) {
        let buffer = &mut self.buffer[range];
        render_fn(buffer);
        // One write per line from the stack: no write buffer on the heap, which is short while
        // the full redraw runs (box art on screen left too little for an 8 KB buffer).
        let mut bytes = [0u8; 2 * 512];
        for chunk in buffer.chunks(512) {
            for (i, x) in chunk.iter().enumerate() {
                bytes[i * 2..i * 2 + 2].copy_from_slice(&x.as_u16().to_le_bytes());
            }
            if let Err(e) = self.writer.write_all(&bytes[..chunk.len() * 2]) {
                self.error.get_or_insert(e);
            }
        }
    }
}

/// Where screenshots asked for over USB go (overwritten each time; read back with the USB
/// control request, see control::REQUEST_SCREENSHOT_READ).
pub const USB_PATH: &str = "/sdcard/system/usb-screenshot.tga";

/// Set while a USB screenshot is being taken.
pub static USB_PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Render a screenshot of the UI and save it to a TGA file on the SD card.
pub fn save_ui_screenshot(
    window: &MinimalSoftwareWindow,
    buffer: &mut [Argb1555],
    usb: bool,
) -> std::io::Result<String> {
    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let filename = if usb { USB_PATH.to_string() } else { format!("/sdcard/screenshot-{}.tga", timestamp) };
    // A full redraw needs room for the renderer's scene: a big enough block, or the memory
    // reserve (crate::reserve) to fall back on. Refuse rather than run out of memory.
    let largest = unsafe {
        esp_idf_svc::sys::heap_caps_get_largest_free_block(esp_idf_svc::sys::MALLOC_CAP_8BIT)
    };
    if largest < 24 * 1024 && !crate::reserve::armed() {
        return Err(std::io::Error::other(format!("not enough memory ({largest} bytes free in one block)")));
    }
    let mut writer = File::create(&filename)?;

    // Write TGA header
    let mut header = [0u8; 18];
    header[2] = 2; // Image type
    header[12..14].copy_from_slice(&(window.size().width as u16).to_le_bytes());
    header[14..16].copy_from_slice(&(window.size().height as u16).to_le_bytes());
    header[16] = 16; // Pixel depth: 16 bits per pixel
    header[17] = 0x20; // Image descriptor (top-left origin)
    writer.write_all(&header)?;

    // Re-render the UI
    window.request_redraw();
    window
        .renderer
        .set_repaint_buffer_type(RepaintBufferType::NewBuffer);
    window.draw_if_needed(|renderer| {
        let mut line_buffer = ScreenshotLineBuffer {
            buffer,
            writer: &mut writer,
            error: None,
        };
        renderer.render_by_line(&mut line_buffer);
        if let Some(e) = line_buffer.error {
            log::error!("Screenshot write error: {e}");
        }
    });

    writer.flush()?;
    Ok(filename)
}
