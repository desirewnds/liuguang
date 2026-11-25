// SPDX-License-Identifier: Apache-2.0
//! Small in-memory thumbnail cache. No browser cache or persistent network state.
use iced::futures::future::{AbortHandle, AbortRegistration};
use iced::widget::image::Handle;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::Cursor,
    time::Duration,
};

#[derive(Default)]
pub struct Images {
    pub ready: HashMap<String, Handle>,
    pending: HashMap<String, AbortHandle>,
    failed: HashSet<String>,
    order: VecDeque<String>,
    context: String,
    pub generation: u64,
}
impl Images {
    pub fn configure(&mut self, context: String) {
        if self.context != context {
            self.cancel();
            self.ready.clear();
            self.failed.clear();
            self.order.clear();
            self.context = context;
        }
    }
    pub fn cancel(&mut self) {
        self.generation += 1;
        for (_, handle) in self.pending.drain() {
            handle.abort();
        }
    }
    pub fn request(
        &mut self,
        urls: impl IntoIterator<Item = String>,
    ) -> Vec<(String, AbortRegistration)> {
        let mut result = Vec::new();
        for url in urls {
            if self.pending.len() >= 2 {
                break;
            }
            if !(url.starts_with("https://") || url.starts_with("http://"))
                || self.ready.contains_key(&url)
                || self.failed.contains(&url)
                || self.pending.contains_key(&url)
            {
                continue;
            }
            let (handle, registration) = AbortHandle::new_pair();
            self.pending.insert(url.clone(), handle);
            result.push((url, registration));
        }
        result
    }
    pub fn finish(&mut self, generation: u64, url: String, result: Result<Handle, String>) {
        if generation != self.generation {
            return;
        }
        self.pending.remove(&url);
        match result {
            Ok(handle) => {
                while self.ready.len() >= 128 {
                    if let Some(old) = self.order.pop_front() {
                        self.ready.remove(&old);
                    } else {
                        break;
                    }
                }
                self.ready.insert(url.clone(), handle);
                self.order.push_back(url);
            }
            Err(_) => {
                if self.failed.len() >= 512 {
                    self.failed.clear()
                }
                self.failed.insert(url);
            }
        }
    }
}
pub async fn load(url: String, mode: String, proxy: String) -> Result<Handle, String> {
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::limited(5));
    if mode == "custom" {
        builder =
            builder.proxy(reqwest::Proxy::all(&proxy).map_err(|_| "代理地址无效".to_string())?);
    } else if mode == "system" {
        // Match the downloader's inherited environment policy; never inspect browser/system proxy settings.
        let env = |lower: &str, upper: &str| {
            std::env::var(lower)
                .ok()
                .or_else(|| std::env::var(upper).ok())
                .filter(|s| !s.is_empty())
        };
        let all = env("all_proxy", "ALL_PROXY");
        for (scheme, address) in [
            (
                "http",
                env("http_proxy", "HTTP_PROXY").or_else(|| all.clone()),
            ),
            ("https", env("https_proxy", "HTTPS_PROXY").or(all)),
        ] {
            if let Some(address) = address {
                let p = if scheme == "https" {
                    reqwest::Proxy::https(address)
                } else {
                    reqwest::Proxy::http(address)
                }
                .map_err(|_| "环境代理地址无效".to_string())?
                .no_proxy(reqwest::NoProxy::from_env());
                builder = builder.proxy(p);
            }
        }
    }
    let client = builder.build().map_err(|e| e.to_string())?;
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    if response
        .content_length()
        .is_some_and(|len| len > 4 * 1024 * 1024)
    {
        return Err("thumbnail too large".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if bytes.len() + chunk.len() > 4 * 1024 * 1024 {
            return Err("thumbnail too large".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    tokio::task::spawn_blocking(move || {
        let mut reader = image_codec::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|e| e.to_string())?;
        let mut limits = image_codec::Limits::default();
        limits.max_alloc = Some(32 * 1024 * 1024);
        limits.max_image_width = Some(8192);
        limits.max_image_height = Some(8192);
        reader.limits(limits);
        let image = reader
            .decode()
            .map_err(|e| e.to_string())?
            .thumbnail(256, 144)
            .into_rgba8();
        let (w, h) = image.dimensions();
        Ok(Handle::from_rgba(w, h, image.into_raw()))
    })
    .await
    .map_err(|e| e.to_string())?
}
impl Drop for Images {
    fn drop(&mut self) {
        self.cancel();
    }
}
