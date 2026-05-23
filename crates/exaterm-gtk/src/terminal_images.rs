use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use base64::Engine as _;
#[cfg(not(test))]
use gdk_pixbuf::prelude::*;
use std::collections::BTreeMap;

const KITTY_MAX_SEQUENCE_BYTES: usize = 16 * 1024 * 1024;
const KITTY_MAX_STORED_IMAGES: usize = 128;
const KITTY_DEFAULT_MAX_COLUMNS: i32 = 40;
const KITTY_DEFAULT_MAX_ROWS: i32 = 20;

#[derive(Default)]
pub struct TerminalImageFilter {
    state: FilterState,
    pending_kitty_images: BTreeMap<u32, PendingKittyImage>,
    stored_kitty_images: BTreeMap<u32, DecodedKittyImage>,
    last_kitty_image_id: Option<u32>,
}

#[derive(Default)]
enum FilterState {
    #[default]
    Ground,
    Escape,
    Apc {
        data: Vec<u8>,
        escape_pending: bool,
    },
}

struct PendingKittyImage {
    params: BTreeMap<String, String>,
    payload: Vec<u8>,
}

#[derive(Clone)]
struct DecodedKittyImage {
    width: i32,
    height: i32,
    pixels: Vec<Pixel>,
}

#[derive(Clone, Copy)]
struct Pixel {
    r: u8,
    g: u8,
    b: u8,
    visible: bool,
}

impl TerminalImageFilter {
    pub fn filter(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(bytes.len());
        for &byte in bytes {
            self.filter_byte(byte, &mut output);
        }
        output
    }

    fn filter_byte(&mut self, byte: u8, output: &mut Vec<u8>) {
        let state = std::mem::replace(&mut self.state, FilterState::Ground);
        self.state = match state {
            FilterState::Ground => {
                if byte == 0x1b {
                    FilterState::Escape
                } else {
                    output.push(byte);
                    FilterState::Ground
                }
            }
            FilterState::Escape => {
                if byte == b'_' {
                    FilterState::Apc {
                        data: Vec::new(),
                        escape_pending: false,
                    }
                } else {
                    output.push(0x1b);
                    output.push(byte);
                    FilterState::Ground
                }
            }
            FilterState::Apc {
                mut data,
                mut escape_pending,
            } => {
                if escape_pending {
                    if byte == b'\\' {
                        let rendered = self.handle_apc(&data);
                        output.extend(rendered.unwrap_or_else(|| original_apc_sequence(&data)));
                        FilterState::Ground
                    } else {
                        data.push(0x1b);
                        data.push(byte);
                        escape_pending = byte == 0x1b;
                        if data.len() > KITTY_MAX_SEQUENCE_BYTES {
                            output.extend(original_apc_sequence(&data));
                            FilterState::Ground
                        } else {
                            FilterState::Apc {
                                data,
                                escape_pending,
                            }
                        }
                    }
                } else if byte == 0x1b {
                    FilterState::Apc {
                        data,
                        escape_pending: true,
                    }
                } else {
                    data.push(byte);
                    if data.len() > KITTY_MAX_SEQUENCE_BYTES {
                        output.extend(original_apc_sequence(&data));
                        FilterState::Ground
                    } else {
                        FilterState::Apc {
                            data,
                            escape_pending,
                        }
                    }
                }
            }
        };
    }

    fn handle_apc(&mut self, data: &[u8]) -> Option<Vec<u8>> {
        let payload = data.strip_prefix(b"G")?;
        let (control, payload) = split_kitty_control_payload(payload);
        let params = parse_kitty_params(control);
        let image_id = self.pending_image_id(&params).unwrap_or(0);
        let more_chunks = params.get("m").is_some_and(|value| value == "1");

        if more_chunks {
            let pending = self
                .pending_kitty_images
                .entry(image_id)
                .or_insert_with(|| PendingKittyImage {
                    params: params.clone(),
                    payload: Vec::new(),
                });
            pending.params.extend(params);
            pending.payload.extend_from_slice(payload);
            return Some(Vec::new());
        }

        let (params, payload) =
            if let Some(mut pending) = self.pending_kitty_images.remove(&image_id) {
                pending.params.extend(params);
                pending.payload.extend_from_slice(payload);
                (pending.params, pending.payload)
            } else {
                (params, payload.to_vec())
            };

        let action = params.get("a").map(String::as_str).unwrap_or("T");
        match action {
            "T" => {
                let image = decode_kitty_image(&params, &payload)?;
                self.store_kitty_image(image_id, image.clone());
                render_image_as_ansi_cells(&image, &params)
            }
            "t" => {
                let image = decode_kitty_image(&params, &payload)?;
                self.store_kitty_image(image_id, image);
                Some(Vec::new())
            }
            "p" => {
                let image_id = kitty_image_id(&params, self.last_kitty_image_id)?;
                let image = self.stored_kitty_images.get(&image_id)?;
                render_image_as_ansi_cells(image, &params)
            }
            _ => Some(Vec::new()),
        }
    }

    fn store_kitty_image(&mut self, image_id: u32, image: DecodedKittyImage) {
        self.last_kitty_image_id = Some(image_id);
        self.stored_kitty_images.insert(image_id, image);
        while self.stored_kitty_images.len() > KITTY_MAX_STORED_IMAGES {
            let Some(oldest_id) = self
                .stored_kitty_images
                .keys()
                .copied()
                .find(|stored_id| *stored_id != image_id)
            else {
                break;
            };
            self.stored_kitty_images.remove(&oldest_id);
        }
    }

    fn pending_image_id(&self, params: &BTreeMap<String, String>) -> Option<u32> {
        kitty_image_id(params, None).or_else(|| {
            (self.pending_kitty_images.len() == 1)
                .then(|| self.pending_kitty_images.keys().next().copied())
                .flatten()
        })
    }
}

fn original_apc_sequence(data: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(data.len() + 4);
    output.extend_from_slice(b"\x1b_");
    output.extend_from_slice(data);
    output.extend_from_slice(b"\x1b\\");
    output
}

fn split_kitty_control_payload(data: &[u8]) -> (&[u8], &[u8]) {
    match data.iter().position(|byte| *byte == b';') {
        Some(index) => (&data[..index], &data[index + 1..]),
        None => (data, &[]),
    }
}

fn parse_kitty_params(control: &[u8]) -> BTreeMap<String, String> {
    String::from_utf8_lossy(control)
        .split(',')
        .filter_map(|part| {
            let (key, value) = part.split_once('=')?;
            (!key.is_empty()).then(|| (key.to_string(), value.to_string()))
        })
        .collect()
}

fn kitty_image_id(params: &BTreeMap<String, String>, fallback: Option<u32>) -> Option<u32> {
    params
        .get("i")
        .and_then(|value| value.parse::<u32>().ok())
        .or(fallback)
}

fn decode_kitty_image(
    params: &BTreeMap<String, String>,
    payload: &[u8],
) -> Option<DecodedKittyImage> {
    let transmission = params.get("t").map(String::as_str).unwrap_or("d");
    if transmission != "d" {
        return None;
    }
    let bytes = decode_base64_payload(payload)?;
    match params.get("f").map(String::as_str).unwrap_or("100") {
        "24" => decode_raw_pixels(&bytes, params, 3),
        "32" => decode_raw_pixels(&bytes, params, 4),
        "100" => decode_png_bytes(&bytes),
        _ => None,
    }
}

fn decode_base64_payload(payload: &[u8]) -> Option<Vec<u8>> {
    let compact = payload
        .iter()
        .copied()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect::<Vec<_>>();
    STANDARD
        .decode(&compact)
        .or_else(|_| STANDARD_NO_PAD.decode(&compact))
        .ok()
}

fn decode_raw_pixels(
    bytes: &[u8],
    params: &BTreeMap<String, String>,
    channels: usize,
) -> Option<DecodedKittyImage> {
    let width = parse_positive_i32_param(params, "s")?;
    let height = parse_positive_i32_param(params, "v")?;
    let pixel_count = width.checked_mul(height)? as usize;
    let expected_len = pixel_count.checked_mul(channels)?;
    if bytes.len() < expected_len {
        return None;
    }

    let mut pixels = Vec::with_capacity(pixel_count);
    for index in 0..pixel_count {
        let offset = index * channels;
        let alpha = if channels == 4 {
            bytes.get(offset + 3).copied().unwrap_or(255)
        } else {
            255
        };
        pixels.push(Pixel {
            r: bytes.get(offset).copied().unwrap_or(0),
            g: bytes.get(offset + 1).copied().unwrap_or(0),
            b: bytes.get(offset + 2).copied().unwrap_or(0),
            visible: alpha >= 32,
        });
    }

    Some(DecodedKittyImage {
        width,
        height,
        pixels,
    })
}

fn parse_positive_i32_param(params: &BTreeMap<String, String>, key: &str) -> Option<i32> {
    params
        .get(key)
        .and_then(|value| value.parse::<i32>().ok())
        .filter(|value| *value > 0)
}

#[cfg(test)]
fn decode_png_bytes(png: &[u8]) -> Option<DecodedKittyImage> {
    (!png.is_empty()).then(|| DecodedKittyImage {
        width: 1,
        height: 1,
        pixels: vec![Pixel {
            r: 255,
            g: 0,
            b: 0,
            visible: true,
        }],
    })
}

#[cfg(not(test))]
fn decode_png_bytes(png: &[u8]) -> Option<DecodedKittyImage> {
    let loader = gdk_pixbuf::PixbufLoader::new();
    loader.write(png).ok()?;
    loader.close().ok()?;
    let pixbuf = loader.pixbuf()?;
    let width = pixbuf.width();
    let height = pixbuf.height();
    if width <= 0 || height <= 0 {
        return None;
    }

    let channels = pixbuf.n_channels() as usize;
    if channels != 3 && channels != 4 {
        return None;
    }
    let rowstride = pixbuf.rowstride() as usize;
    let pixels = pixbuf.read_pixel_bytes();
    let pixels = pixels.as_ref();
    let mut decoded_pixels = Vec::with_capacity(width.checked_mul(height)? as usize);

    for source_y in 0..height as usize {
        for source_x in 0..width as usize {
            let offset = source_y * rowstride + source_x * channels;
            let alpha = if channels == 4 {
                pixels.get(offset + 3).copied().unwrap_or(255)
            } else {
                255
            };
            decoded_pixels.push(Pixel {
                r: pixels.get(offset).copied().unwrap_or(0),
                g: pixels.get(offset + 1).copied().unwrap_or(0),
                b: pixels.get(offset + 2).copied().unwrap_or(0),
                visible: alpha >= 32,
            });
        }
    }

    Some(DecodedKittyImage {
        width,
        height,
        pixels: decoded_pixels,
    })
}

fn render_image_as_ansi_cells(
    image: &DecodedKittyImage,
    params: &BTreeMap<String, String>,
) -> Option<Vec<u8>> {
    if image.width <= 0 || image.height <= 0 || image.pixels.is_empty() {
        return None;
    }

    let target_cols = target_columns(params, image.width);
    let target_rows = target_rows(params, image.width, image.height, target_cols);

    let mut output = Vec::new();
    output.extend_from_slice(b"\x1b7");
    for row in 0..target_rows {
        if row > 0 {
            output.extend_from_slice(b"\r\n");
        }
        for col in 0..target_cols {
            let top = sample_pixel(image, col, row * 2, target_cols, target_rows * 2);
            let bottom = sample_pixel(image, col, row * 2 + 1, target_cols, target_rows * 2);
            append_half_block(&mut output, top, bottom);
        }
        output.extend_from_slice(b"\x1b[0m");
    }
    output.extend_from_slice(b"\x1b8");
    Some(output)
}

fn target_columns(params: &BTreeMap<String, String>, image_width: i32) -> i32 {
    params
        .get("c")
        .and_then(|value| value.parse::<i32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or_else(|| (image_width / 8).clamp(1, KITTY_DEFAULT_MAX_COLUMNS))
        .clamp(1, KITTY_DEFAULT_MAX_COLUMNS)
}

fn target_rows(
    params: &BTreeMap<String, String>,
    image_width: i32,
    image_height: i32,
    target_cols: i32,
) -> i32 {
    params
        .get("r")
        .and_then(|value| value.parse::<i32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or_else(|| {
            let ratio = image_height as f64 / image_width as f64;
            ((target_cols as f64 * ratio) / 2.0).ceil() as i32
        })
        .clamp(1, KITTY_DEFAULT_MAX_ROWS)
}

fn sample_pixel(
    image: &DecodedKittyImage,
    target_x: i32,
    target_y: i32,
    target_width: i32,
    target_height: i32,
) -> Pixel {
    let source_x = ((target_x as i64 * image.width as i64) / target_width as i64)
        .clamp(0, image.width as i64 - 1) as usize;
    let source_y = ((target_y as i64 * image.height as i64) / target_height as i64)
        .clamp(0, image.height as i64 - 1) as usize;
    let offset = source_y * image.width as usize + source_x;
    image.pixels.get(offset).copied().unwrap_or(Pixel {
        r: 0,
        g: 0,
        b: 0,
        visible: false,
    })
}

fn append_half_block(output: &mut Vec<u8>, top: Pixel, bottom: Pixel) {
    match (top.visible, bottom.visible) {
        (false, false) => {
            output.extend_from_slice(b"\x1b[0m ");
        }
        (true, false) => {
            append_fg(output, top);
            output.extend_from_slice(b"\x1b[49m");
            output.extend_from_slice("▀".as_bytes());
        }
        (false, true) => {
            append_fg(output, bottom);
            output.extend_from_slice(b"\x1b[49m");
            output.extend_from_slice("▄".as_bytes());
        }
        (true, true) => {
            append_fg(output, top);
            append_bg(output, bottom);
            output.extend_from_slice("▀".as_bytes());
        }
    }
}

fn append_fg(output: &mut Vec<u8>, pixel: Pixel) {
    output.extend_from_slice(format!("\x1b[38;2;{};{};{}m", pixel.r, pixel.g, pixel.b).as_bytes());
}

fn append_bg(output: &mut Vec<u8>, pixel: Pixel) {
    output.extend_from_slice(format!("\x1b[48;2;{};{};{}m", pixel.r, pixel.g, pixel.b).as_bytes());
}

#[cfg(test)]
mod tests {
    use super::TerminalImageFilter;
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;

    const PNG_1X1_RED: &str =
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADUlEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC";

    #[test]
    fn kitty_png_payload_is_rendered_as_ansi_cells() {
        let mut filter = TerminalImageFilter::default();
        let input = format!("\x1b_Ga=T,f=100,t=d,c=1,r=1;{PNG_1X1_RED}\x1b\\");
        let output = filter.filter(input.as_bytes());
        let output = String::from_utf8(output).expect("ansi output should be utf-8");

        assert!(output.contains("\x1b7"));
        assert!(output.contains("\x1b[38;2;255;0;0m"));
        assert!(output.contains("▀"));
        assert!(output.contains("\x1b8"));
    }

    #[test]
    fn split_kitty_chunks_are_combined() {
        let mut filter = TerminalImageFilter::default();
        let split_at = PNG_1X1_RED.len() / 2;
        let first = format!(
            "\x1b_Ga=T,f=100,t=d,i=7,m=1;{}\x1b\\",
            &PNG_1X1_RED[..split_at]
        );
        let second = format!("\x1b_Gi=7,m=0;{}\x1b\\", &PNG_1X1_RED[split_at..]);

        assert!(filter.filter(first.as_bytes()).is_empty());
        let output = String::from_utf8(filter.filter(second.as_bytes()))
            .expect("ansi output should be utf-8");
        assert!(output.contains("\x1b[38;2;255;0;0m"));
    }

    #[test]
    fn split_kitty_chunks_can_omit_repeated_image_id() {
        let mut filter = TerminalImageFilter::default();
        let split_at = PNG_1X1_RED.len() / 2;
        let first = format!(
            "\x1b_Ga=T,f=100,t=d,i=7,m=1;{}\x1b\\",
            &PNG_1X1_RED[..split_at]
        );
        let second = format!("\x1b_Gm=0;{}\x1b\\", &PNG_1X1_RED[split_at..]);

        assert!(filter.filter(first.as_bytes()).is_empty());
        let output = String::from_utf8(filter.filter(second.as_bytes()))
            .expect("ansi output should be utf-8");
        assert!(output.contains("\x1b[38;2;255;0;0m"));
    }

    #[test]
    fn kitty_raw_rgba_payload_is_rendered_as_ansi_cells() {
        let mut filter = TerminalImageFilter::default();
        let rgba = STANDARD.encode([255, 0, 0, 255, 0, 0, 255, 255]);
        let input = format!("\x1b_Ga=T,f=32,t=d,s=1,v=2,c=1,r=1;{rgba}\x1b\\");
        let output = String::from_utf8(filter.filter(input.as_bytes()))
            .expect("ansi output should be utf-8");

        assert!(output.contains("\x1b[38;2;255;0;0m"));
        assert!(output.contains("\x1b[48;2;0;0;255m"));
        assert!(output.contains("▀"));
    }

    #[test]
    fn kitty_transmit_then_place_renders_stored_image() {
        let mut filter = TerminalImageFilter::default();
        let rgb = STANDARD.encode([255, 0, 0, 0, 0, 255]);
        let transmit = format!("\x1b_Ga=t,f=24,t=d,s=1,v=2,i=42;{rgb}\x1b\\");
        let place = "\x1b_Ga=p,i=42,c=1,r=1;\x1b\\";

        assert!(filter.filter(transmit.as_bytes()).is_empty());
        let output = String::from_utf8(filter.filter(place.as_bytes()))
            .expect("ansi output should be utf-8");

        assert!(output.contains("\x1b[38;2;255;0;0m"));
        assert!(output.contains("\x1b[48;2;0;0;255m"));
        assert!(output.contains("▀"));
    }

    #[test]
    fn transparent_pixels_reset_cell_background() {
        let mut filter = TerminalImageFilter::default();
        let rgba = STANDARD.encode([255, 0, 0, 255, 0, 0, 0, 0, 0, 0, 255, 255, 0, 0, 0, 0]);
        let input = format!("\x1b_Ga=T,f=32,t=d,s=2,v=2,c=2,r=1;{rgba}\x1b\\");
        let output = String::from_utf8(filter.filter(input.as_bytes()))
            .expect("ansi output should be utf-8");

        assert!(output.contains("▀\x1b[0m "));
    }
}
