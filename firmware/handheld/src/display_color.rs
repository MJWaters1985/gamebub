//! Display color settings (Settings > Display), applied to everything on screen: games through
//! the core's color correction, and the menus as they're sent to the FPGA.

use std::cell::RefCell;

use crate::kvs::keys;

pub const TEMPERATURE_CHOICES: [&str; 7] = [
    "Coolest", "Cooler", "Cool", "Neutral", "Warm", "Warmer", "Warmest",
];
pub const SATURATION_CHOICES: [&str; 7] = ["0%", "25%", "50%", "75%", "100%", "125%", "150%"];
pub const CONTRAST_CHOICES: [&str; 5] = ["80%", "90%", "100%", "110%", "120%"];
pub const GAMMA_CHOICES: [&str; 3] = ["Darker", "Default", "Brighter"];
pub const GAIN_CHOICES: [&str; 5] = ["80%", "85%", "90%", "95%", "100%"];

/// Red, green and blue gains for each color temperature step.
const TEMPERATURE_GAINS: [[f32; 3]; 7] = [
    [0.80, 0.90, 1.0],
    [0.87, 0.94, 1.0],
    [0.94, 0.97, 1.0],
    [1.0, 1.0, 1.0],
    [1.0, 0.95, 0.85],
    [1.0, 0.90, 0.74],
    [1.0, 0.85, 0.62],
];

/// Luma weights (Rec. 709) used for saturation.
const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// The user's display color settings as a color transform: a 3x3 matrix, then a tone curve.
#[derive(Clone, Copy, PartialEq)]
pub struct DisplayColor {
    /// [Red] [Green] [Blue]
    pub matrix: [f32; 9],
    contrast: f32,
    gamma: f32,
}

impl DisplayColor {
    pub const NEUTRAL: DisplayColor = DisplayColor {
        matrix: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        contrast: 1.0,
        gamma: 1.0,
    };

    /// The current settings.
    pub fn current() -> DisplayColor {
        let index = |key: &crate::kvs::KvsKey<i32>, len: usize| {
            key.get().unwrap_or(0).clamp(0, len as i32 - 1) as usize
        };

        let temperature =
            TEMPERATURE_GAINS[index(&keys::DISPLAY_TEMPERATURE, TEMPERATURE_GAINS.len())];
        let gain = |key| 0.80 + 0.05 * index(key, GAIN_CHOICES.len()) as f32;
        let gains = [
            temperature[0] * gain(&keys::DISPLAY_RED),
            temperature[1] * gain(&keys::DISPLAY_GREEN),
            temperature[2] * gain(&keys::DISPLAY_BLUE),
        ];
        let saturation = 0.25 * index(&keys::DISPLAY_SATURATION, SATURATION_CHOICES.len()) as f32;

        // Saturation (mix with luma), then the per-channel gains.
        let mut matrix = [0.0; 9];
        for row in 0..3 {
            for col in 0..3 {
                let identity = if row == col { 1.0 } else { 0.0 };
                matrix[row * 3 + col] =
                    gains[row] * (saturation * identity + (1.0 - saturation) * LUMA[col]);
            }
        }

        DisplayColor {
            matrix,
            contrast: 0.8 + 0.1 * index(&keys::DISPLAY_CONTRAST, CONTRAST_CHOICES.len()) as f32,
            gamma: [1.2, 1.0, 0.83][index(&keys::DISPLAY_GAMMA, GAMMA_CHOICES.len())],
        }
    }

    /// Apply the tone curve (contrast and gamma) to a value from 0.0 to 1.0.
    pub fn curve(&self, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0).powf(self.gamma);
        ((x - 0.5) * self.contrast + 0.5).clamp(0.0, 1.0)
    }

    /// `self.matrix * other` (applies `other` first).
    pub fn matrix_after(&self, other: &[f32; 9]) -> [f32; 9] {
        let mut out = [0.0; 9];
        for row in 0..3 {
            for col in 0..3 {
                out[row * 3 + col] = (0..3)
                    .map(|k| self.matrix[row * 3 + k] * other[k * 3 + col])
                    .sum();
            }
        }
        out
    }
}

/// Whether the settings leave colors unchanged.
pub fn is_neutral() -> bool {
    DisplayColor::current() == DisplayColor::NEUTRAL
}

/// Reset all display color settings to their defaults.
pub fn reset() {
    keys::DISPLAY_TEMPERATURE.set(&3);
    keys::DISPLAY_SATURATION.set(&4);
    keys::DISPLAY_CONTRAST.set(&2);
    keys::DISPLAY_GAMMA.set(&1);
    keys::DISPLAY_RED.set(&4);
    keys::DISPLAY_GREEN.set(&4);
    keys::DISPLAY_BLUE.set(&4);
}

/// The menu color transform, in fixed point for 5-bit channels.
struct MenuTransform {
    /// Matrix in 1/4096ths.
    matrix: [i32; 9],
    /// Tone curve from matrix output (0..=MENU_LUT_MAX) to a 5-bit channel.
    lut: [u8; 1024],
}

/// Matrix output for full intensity: 31 * 4096 >> 7.
const MENU_LUT_MAX: usize = 992;

thread_local! {
    /// The menu transform (UI thread only), or `None` when the settings are neutral.
    static MENU_TRANSFORM: RefCell<Option<Box<MenuTransform>>> = const { RefCell::new(None) };
}

/// Update the menu transform from the current settings. Call on the UI thread; the screen needs
/// a full redraw afterwards.
pub fn apply_to_menus() {
    let color = DisplayColor::current();
    let transform = if color == DisplayColor::NEUTRAL {
        None
    } else {
        let mut lut = [0u8; 1024];
        for (i, entry) in lut.iter_mut().enumerate() {
            let x = (i.min(MENU_LUT_MAX) as f32) / (MENU_LUT_MAX as f32);
            *entry = (color.curve(x) * 31.0 + 0.5) as u8;
        }
        Some(Box::new(MenuTransform {
            matrix: color.matrix.map(|x| (x * 4096.0).round() as i32),
            lut,
        }))
    };
    MENU_TRANSFORM.with(|cell| *cell.borrow_mut() = transform);
}

/// Apply the menu transform to a line of ARGB1555 pixels, from `input` into `output`. Returns
/// `input` itself when no transform is needed.
pub fn transform_menu_line<'a>(input: &'a [u16], output: &'a mut [u16]) -> &'a [u16] {
    MENU_TRANSFORM.with(|cell| {
        let cell = cell.borrow();
        let Some(transform) = cell.as_ref() else {
            return input;
        };
        let m = &transform.matrix;
        let lut = &transform.lut;
        let channel = |r: i32, g: i32, b: i32, row: usize| {
            let value = (m[row * 3] * r + m[row * 3 + 1] * g + m[row * 3 + 2] * b) >> 7;
            lut[value.clamp(0, 1023) as usize] as u16
        };
        for (out, pixel) in output.iter_mut().zip(input) {
            let raw = *pixel;
            if raw & 0x8000 == 0 {
                // Transparent: the game shows through.
                *out = *pixel;
                continue;
            }
            let r = ((raw >> 10) & 0x1F) as i32;
            let g = ((raw >> 5) & 0x1F) as i32;
            let b = (raw & 0x1F) as i32;
            *out = 0x8000
                | (channel(r, g, b, 0) << 10)
                | (channel(r, g, b, 1) << 5)
                | channel(r, g, b, 2);
        }
        &output[..input.len()]
    })
}
