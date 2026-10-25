//! Loading covers for egui: downloads go through one shared HTTP client on
//! Glass's runtime and decoding happens two images at a time, where
//! egui_extras would start a thread (and a new connection) for every image.
//!
//! egui keeps a cover's bytes and decoded pixels only until its texture is
//! made (`reduce_texture_memory`), so each cover is held once, on the GPU.

use eframe::egui::load::{Bytes, BytesLoadResult, BytesLoader, BytesPoll, ImageLoadResult, ImageLoader, ImagePoll, LoadError, SizeHint};
use eframe::egui::mutex::Mutex;
use eframe::egui::{self, ColorImage};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;

/// Downloads at once, and decodes at once.
const DOWNLOADS: usize = 6;
const DECODES: usize = 2;
/// Covers are drawn at most this many pixels across; bigger ones (artwork
/// embedded in local files can be 3000 px) are scaled down once decoded.
const MAX_SIDE: u32 = 1024;
/// Anything bigger isn't a cover.
const MAX_BYTES: usize = 16 << 20;

pub fn install(ctx: &egui::Context, rt: tokio::runtime::Handle) {
    ctx.options_mut(|o| o.reduce_texture_memory = true);
    ctx.add_bytes_loader(Arc::new(HttpLoader::new(rt.clone())));
    ctx.add_image_loader(Arc::new(Decoder::new(rt)));
}

enum Entry<T> {
    Pending,
    Ready(T),
    Failed(String),
}

type Cache<T> = Arc<Mutex<HashMap<String, Entry<T>>>>;

/// Stores a finished load unless the URI was forgotten meanwhile, then
/// repaints (after unlocking: egui may lock the cache from inside the context).
fn finish<T>(cache: &Cache<T>, ctx: &egui::Context, uri: &str, result: Result<T, String>) {
    let stored = match cache.lock().get_mut(uri) {
        Some(e @ Entry::Pending) => {
            *e = match result {
                Ok(v) => Entry::Ready(v),
                Err(why) => Entry::Failed(why),
            };
            true
        }
        _ => false,
    };
    if stored {
        ctx.request_repaint();
    }
}

/// `http(s)://` images.
struct HttpLoader {
    rt: tokio::runtime::Handle,
    http: reqwest::Client,
    slots: Arc<Semaphore>,
    cache: Cache<(Arc<[u8]>, Option<String>)>,
}

impl HttpLoader {
    fn new(rt: tokio::runtime::Handle) -> Self {
        let http = crate::config::http().connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(30)).build().unwrap_or_default();
        Self { rt, http, slots: Arc::new(Semaphore::new(DOWNLOADS)), cache: Default::default() }
    }
}

async fn download(http: &reqwest::Client, uri: &str) -> Result<(Arc<[u8]>, Option<String>), String> {
    let resp = http.get(uri).send().await.and_then(|r| r.error_for_status()).map_err(|e| e.without_url().to_string())?;
    if resp.content_length().is_some_and(|n| n as usize > MAX_BYTES) {
        return Err("image too large".into());
    }
    let mime = resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(str::to_string);
    let bytes = resp.bytes().await.map_err(|e| e.without_url().to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err("image too large".into());
    }
    Ok((Arc::from(&bytes[..]), mime))
}

impl BytesLoader for HttpLoader {
    fn id(&self) -> &str {
        egui::generate_loader_id!(HttpLoader)
    }

    fn load(&self, ctx: &egui::Context, uri: &str) -> BytesLoadResult {
        if !uri.starts_with("https://") && !uri.starts_with("http://") {
            return Err(LoadError::NotSupported);
        }
        let mut cache = self.cache.lock();
        match cache.get(uri) {
            Some(Entry::Ready((bytes, mime))) => return Ok(BytesPoll::Ready { size: None, bytes: Bytes::Shared(bytes.clone()), mime: mime.clone() }),
            Some(Entry::Failed(why)) => return Err(LoadError::Loading(why.clone())),
            Some(Entry::Pending) => return Ok(BytesPoll::Pending { size: None }),
            None => {}
        }
        cache.insert(uri.to_string(), Entry::Pending);
        drop(cache);
        let (http, slots, cache, ctx, uri) = (self.http.clone(), self.slots.clone(), self.cache.clone(), ctx.clone(), uri.to_string());
        self.rt.spawn(async move {
            let Ok(_slot) = slots.acquire_owned().await else { return };
            // Scrolled past and forgotten while it waited its turn.
            if !matches!(cache.lock().get(&uri), Some(Entry::Pending)) {
                return;
            }
            let result = download(&http, &uri).await;
            if let Err(why) = &result {
                log::debug!("cover {uri}: {why}");
            }
            finish(&cache, &ctx, &uri, result);
        });
        Ok(BytesPoll::Pending { size: None })
    }

    fn forget(&self, uri: &str) {
        self.cache.lock().remove(uri);
    }

    fn forget_all(&self) {
        self.cache.lock().clear();
    }

    fn byte_size(&self) -> usize {
        self.cache.lock().values().map(|e| if let Entry::Ready((b, _)) = e { b.len() } else { 0 }).sum()
    }

    fn has_pending(&self) -> bool {
        self.cache.lock().values().any(|e| matches!(e, Entry::Pending))
    }
}

/// Decodes whatever bytes the loaders above (or `include_bytes`) provide.
struct Decoder {
    rt: tokio::runtime::Handle,
    slots: Arc<Semaphore>,
    cache: Cache<Arc<ColorImage>>,
}

impl Decoder {
    fn new(rt: tokio::runtime::Handle) -> Self {
        Self { rt, slots: Arc::new(Semaphore::new(DECODES)), cache: Default::default() }
    }
}

fn decode(bytes: &[u8]) -> Result<ColorImage, String> {
    use image::ImageDecoder;
    let format = image::guess_format(bytes).map_err(|e| e.to_string())?;
    let mut decoder = image::ImageReader::with_format(std::io::Cursor::new(bytes), format).into_decoder().map_err(|e| e.to_string())?;
    let orientation = decoder.orientation().map_err(|e| e.to_string())?;
    let mut img = image::DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    img.apply_orientation(orientation);
    let source = egui::vec2(img.width() as f32, img.height() as f32);
    if img.width().max(img.height()) > MAX_SIDE {
        img = img.resize(MAX_SIDE, MAX_SIDE, image::imageops::FilterType::Triangle);
    }
    let rgba = img.into_rgba8();
    let mut out = ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw());
    out.source_size = source;
    Ok(out)
}

impl ImageLoader for Decoder {
    fn id(&self) -> &str {
        egui::generate_loader_id!(Decoder)
    }

    fn load(&self, ctx: &egui::Context, uri: &str, _: SizeHint) -> ImageLoadResult {
        match self.cache.lock().get(uri) {
            Some(Entry::Ready(img)) => return Ok(ImagePoll::Ready { image: img.clone() }),
            Some(Entry::Failed(why)) => return Err(LoadError::Loading(why.clone())),
            Some(Entry::Pending) => return Ok(ImagePoll::Pending { size: None }),
            None => {}
        }
        let bytes = match ctx.try_load_bytes(uri)? {
            BytesPoll::Ready { bytes, .. } => bytes,
            BytesPoll::Pending { size } => return Ok(ImagePoll::Pending { size }),
        };
        let bytes: Arc<[u8]> = match bytes {
            Bytes::Static(b) => Arc::from(b),
            Bytes::Shared(b) => b,
        };
        self.cache.lock().insert(uri.to_string(), Entry::Pending);
        let (slots, cache, ctx, uri) = (self.slots.clone(), self.cache.clone(), ctx.clone(), uri.to_string());
        self.rt.spawn(async move {
            let Ok(_slot) = slots.acquire_owned().await else { return };
            if !matches!(cache.lock().get(&uri), Some(Entry::Pending)) {
                return;
            }
            let result = tokio::task::spawn_blocking(move || decode(&bytes)).await.unwrap_or_else(|e| Err(e.to_string()));
            finish(&cache, &ctx, &uri, result.map(Arc::new));
        });
        Ok(ImagePoll::Pending { size: None })
    }

    fn forget(&self, uri: &str) {
        self.cache.lock().remove(uri);
    }

    fn forget_all(&self) {
        self.cache.lock().clear();
    }

    fn byte_size(&self) -> usize {
        self.cache.lock().values().map(|e| if let Entry::Ready(img) = e { img.pixels.len() * 4 } else { 0 }).sum()
    }

    fn has_pending(&self) -> bool {
        self.cache.lock().values().any(|e| matches!(e, Entry::Pending))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn decodes_and_scales_down_big_covers() {
        let mut png = Vec::new();
        image::RgbImage::from_pixel(2048, 1024, image::Rgb([200, 40, 90])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let img = super::decode(&png).unwrap();
        assert_eq!(img.size, [1024, 512]);
        assert_eq!(img.source_size, eframe::egui::vec2(2048.0, 1024.0));
        assert!(super::decode(b"not an image").is_err());
    }
}
