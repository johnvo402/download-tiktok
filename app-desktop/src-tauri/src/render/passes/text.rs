use super::super::*;
impl Renderer {
    pub(in crate::render::renderer) fn cache_text(
        &mut self,
        key: String,
        text: &str,
        rect: PixelRect,
        mut style: TextStyle,
        minimum: u16,
    ) -> Result<String, String> {
        // Include content and styling: a future preview scene edit must never reuse stale pixels.
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hash);
        format!("{style:?}").hash(&mut hash);
        (rect.x, rect.y, rect.width, rect.height, minimum).hash(&mut hash);
        let key = format!("{key}:{:016x}", hash.finish());
        if self.text.contains_key(&key) {
            self.lru.retain(|v| v != &key);
            self.lru.push_back(key.clone());
            return Ok(key);
        }
        let bytes = loop {
            match self.raster.rasterize(text, rect, &style) {
                Ok(bytes) => break bytes,
                Err(error) if style.size > minimum => {
                    style.size -= 1;
                    log::debug!("Subtitle font fit: {}px ({error})", style.size);
                }
                Err(error) => return Err(error),
            }
        };
        while self.text_bytes + bytes.len() > 64 * 1024 * 1024 || self.text.len() >= 64 {
            let Some(old) = self.lru.pop_front() else {
                break;
            };
            if let Some(layer) = self.text.remove(&old) {
                self.text_bytes = self.text_bytes.saturating_sub(layer.bytes);
            }
        }
        let texture = Arc::new(Texture::new(
            &self.gpu.device,
            rect.width,
            rect.height,
            "cached text layer",
        ));
        texture.upload(&self.gpu.queue, &bytes);
        let bind = Self::layer_bind(
            &self.gpu.device,
            &self.layout,
            &self.sampler,
            &texture,
            rect,
            1.0,
        );
        self.text_bytes += bytes.len();
        self.text.insert(
            key.clone(),
            Layer {
                _texture: texture,
                bind,
                rect,
                bytes: bytes.len(),
            },
        );
        self.lru.push_back(key.clone());
        Ok(key)
    }
}
