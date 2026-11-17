use std::{
    fs::File,
    io::{BufWriter, Write},
    time::SystemTime,
};

use slint::platform::software_renderer::{LineBufferProvider, RepaintBufferType};

use crate::ui::slint::{Argb1555, MinimalSoftwareWindow};

struct ScreenshotLineBuffer<'a> {
    buffer: &'a mut [Argb1555],
    writer: &'a mut BufWriter<File>,
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
        for x in buffer {
            let _ = self.writer.write_all(&x.as_u16().to_le_bytes());
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
    let file = File::create(&filename)?;
    let mut writer = BufWriter::new(file);

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
        };
        renderer.render_by_line(&mut line_buffer);
    });

    writer.flush()?;
    Ok(filename)
}
