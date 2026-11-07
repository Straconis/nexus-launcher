pub(super) const S3D_BANNER_URL: &str =
    "https://straconis.dev/wp-content/uploads/2026/11/s3d-labs-tile-banner.png";

use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use eframe::egui;
use image::{ImageFormat, ImageReader, Limits};

enum Banner {
    Loading,
    Ready(egui::TextureHandle),
    Failed,
}

type LoadedBanner = (String, Option<egui::ColorImage>);

pub(super) struct TileBanners {
    queue: Sender<String>,
    results: Receiver<LoadedBanner>,
    images: HashMap<String, Banner>,
}

impl TileBanners {
    pub(super) fn new() -> Self {
        let (queue, requests) = mpsc::channel::<String>();
        let (results, received) = mpsc::channel();
        std::thread::spawn(move || {
            let Ok(client) = reqwest::blocking::Client::builder()
                .https_only(true)
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(8))
                .build()
            else {
                return;
            };
            while let Ok(url) = requests.recv() {
                let image = load(&client, &url);
                if results.send((url, image)).is_err() {
                    break;
                }
            }
        });
        Self {
            queue,
            results: received,
            images: HashMap::new(),
        }
    }

    pub(super) fn update<'a>(
        &mut self,
        ctx: &egui::Context,
        urls: impl Iterator<Item = (&'a str, bool)>,
    ) {
        let mut active = HashSet::new();
        let mut pending = self
            .images
            .values()
            .filter(|image| matches!(image, Banner::Loading))
            .count();
        for (url, visible) in urls.filter(|(url, _)| !url.trim().is_empty()) {
            active.insert(url);
            if visible && pending < 4 && !self.images.contains_key(url) {
                pending += 1;
                let state = if self.queue.send(url.to_owned()).is_ok() {
                    Banner::Loading
                } else {
                    Banner::Failed
                };
                self.images.insert(url.to_owned(), state);
            }
        }
        self.images.retain(|url, _| active.contains(url.as_str()));
        while let Ok((url, image)) = self.results.try_recv() {
            if let Some(state) = self.images.get_mut(&url) {
                *state = match image {
                    Some(image) => Banner::Ready(ctx.load_texture(
                        "tile banner",
                        image,
                        egui::TextureOptions::LINEAR,
                    )),
                    None => Banner::Failed,
                };
            }
        }
        if self
            .images
            .values()
            .any(|image| matches!(image, Banner::Loading))
        {
            ctx.request_repaint_after(Duration::from_millis(150));
        }
    }

    pub(super) fn retry_failed(&mut self) {
        self.images
            .retain(|_, state| !matches!(state, Banner::Failed));
    }

    pub(super) fn get(&self, url: &str) -> Option<&egui::TextureHandle> {
        match self.images.get(url) {
            Some(Banner::Ready(texture)) => Some(texture),
            _ => None,
        }
    }
}

fn load(client: &reqwest::blocking::Client, url: &str) -> Option<egui::ColorImage> {
    const MAX_BYTES: u64 = 4 * 1024 * 1024;
    let url = reqwest::Url::parse(url.trim()).ok()?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    let response = client.get(url).send().ok()?.error_for_status().ok()?;
    if !response.status().is_success() || response.content_length().is_some_and(|n| n > MAX_BYTES) {
        return None;
    }
    let mut bytes = Vec::new();
    response.take(MAX_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_BYTES {
        return None;
    }
    decode(&bytes)
}

fn decode(bytes: &[u8]) -> Option<egui::ColorImage> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    if !matches!(reader.format(), Some(ImageFormat::Png | ImageFormat::Jpeg)) {
        return None;
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let image = reader
        .decode()
        .ok()?
        .resize_to_fill(768, 256, image::imageops::FilterType::Triangle)
        .into_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied(
        [768, 256],
        image.as_raw(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banner_decoding_crops_and_rejects_invalid_or_oversized_images() {
        let decoded = decode(include_bytes!(
            "../assets/branding/s3d-labs-tile-banner.png"
        ))
        .unwrap();
        assert_eq!(decoded.size, [768, 256]);
        assert!(decode(b"not an image").is_none());
        let mut oversized = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(4097, 1)
            .write_to(&mut oversized, ImageFormat::Png)
            .unwrap();
        assert!(decode(oversized.get_ref()).is_none());
    }
}
