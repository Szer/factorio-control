use super::*;
use std::{
    hash::{Hash, Hasher},
    sync::OnceLock,
    time::Instant,
};

const TTL: Duration = Duration::from_secs(10 * 60);
const MAX_BYTES: usize = 32 * 1024 * 1024;
struct Entry {
    fetched: Instant,
    bytes: usize,
    releases: Vec<Release>,
}
pub(super) struct Cache {
    entries: tokio::sync::Mutex<BTreeMap<String, Entry>>,
    gates: [tokio::sync::Mutex<()>; 8],
    ttl: Duration,
    max_bytes: usize,
}
impl Cache {
    fn new(ttl: Duration, max_bytes: usize) -> Self {
        Self {
            entries: Default::default(),
            gates: std::array::from_fn(|_| Default::default()),
            ttl,
            max_bytes,
        }
    }
    async fn lookup(&self, key: &str, requested: Instant, refresh: bool) -> Option<Vec<Release>> {
        self.entries
            .lock()
            .await
            .get(key)
            .filter(|entry| {
                entry.fetched.elapsed() < self.ttl && (!refresh || entry.fetched >= requested)
            })
            .map(|entry| entry.releases.clone())
    }
    pub(super) async fn get(
        &self,
        client: &reqwest::Client,
        name: &str,
        url: &str,
        refresh: bool,
    ) -> Result<Vec<Release>> {
        let requested = Instant::now();
        if let Some(value) = self.lookup(url, requested, refresh).await {
            return Ok(value);
        }
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        url.hash(&mut hash);
        let _gate = self.gates[(hash.finish() % self.gates.len() as u64) as usize]
            .lock()
            .await;
        if let Some(value) = self.lookup(url, requested, refresh).await {
            return Ok(value);
        }
        let _permit = UPDATE_CHECKS
            .acquire()
            .await
            .map_err(|_| Error::bad("Portal unavailable"))?;
        let response = client
            .get(url)
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .map_err(|_| Error {
                status: StatusCode::BAD_GATEWAY,
                message: "Could not reach the Factorio mod portal".into(),
            })?;
        let bytes = response_bytes(response, 4 * 1024 * 1024).await?;
        let document: PortalMod = serde_json::from_slice(&bytes)
            .map_err(|_| Error::bad("Invalid mod portal response"))?;
        if document.name != name || document.releases.len() > 10_000 {
            return Err(Error::bad("Unexpected mod portal response"));
        }
        let mut entries = self.entries.lock().await;
        entries.retain(|_, entry| entry.fetched.elapsed() < self.ttl);
        entries.remove(url);
        let mut used: usize = entries.values().map(|entry| entry.bytes).sum();
        while used.saturating_add(bytes.len()) > self.max_bytes || entries.len() >= MAX_MODS {
            let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, entry)| entry.fetched)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some(entry) = entries.remove(&oldest) {
                used = used.saturating_sub(entry.bytes);
            }
        }
        if bytes.len() <= self.max_bytes {
            entries.insert(
                url.into(),
                Entry {
                    fetched: Instant::now(),
                    bytes: bytes.len(),
                    releases: document.releases.clone(),
                },
            );
        }
        Ok(document.releases)
    }
}
fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(|| Cache::new(TTL, MAX_BYTES))
}
#[cfg(test)]
pub(super) async fn prime(name: &str, releases: Vec<Release>) {
    cache().entries.lock().await.insert(
        format!("https://mods.factorio.com/api/mods/{name}/full"),
        Entry {
            fetched: Instant::now(),
            bytes: 1024,
            releases,
        },
    );
}
pub(super) async fn get(
    client: &reqwest::Client,
    name: &str,
    full: bool,
    refresh: bool,
) -> Result<Vec<Release>> {
    if !valid_name(name) {
        return Err(Error::bad("Invalid mod name"));
    }
    let suffix = if full { "/full" } else { "" };
    cache()
        .get(
            client,
            name,
            &format!("https://mods.factorio.com/api/mods/{name}{suffix}"),
            refresh,
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    #[tokio::test]
    async fn coalesces_concurrent_http_checks_and_refreshes_expired_or_forced_entries()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let server = axum::Router::new().route(
            "/metadata",
            axum::routing::get(move || {
                let count = count.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(30)).await;
                    Json(serde_json::json!({"name":"fixture","releases":[]}))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/metadata", listener.local_addr()?);
        let server = tokio::spawn(async move { axum::serve(listener, server).await });
        let cache = Arc::new(Cache::new(Duration::from_secs(600), MAX_BYTES));
        let client = reqwest::Client::new();
        let mut checks = tokio::task::JoinSet::new();
        for _ in 0..12 {
            let cache = cache.clone();
            let url = url.clone();
            let client = client.clone();
            checks.spawn(async move { cache.get(&client, "fixture", &url, false).await });
        }
        while let Some(check) = checks.join_next().await {
            check?.map_err(|error| error.message)?;
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        cache
            .get(&client, "fixture", &url, false)
            .await
            .map_err(|e| e.message)?;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        cache
            .get(&client, "fixture", &url, true)
            .await
            .map_err(|e| e.message)?;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        for entry in cache.entries.lock().await.values_mut() {
            entry.fetched = Instant::now() - Duration::from_secs(601);
        }
        cache
            .get(&client, "fixture", &url, false)
            .await
            .map_err(|e| e.message)?;
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        server.abort();
        Ok(())
    }
    #[tokio::test]
    async fn errors_are_retried_and_cache_storage_is_bounded()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let server = axum::Router::new().route(
            "/{name}",
            axum::routing::get(move |Path(name): Path<String>| {
                let count = count.clone();
                async move {
                    if count.fetch_add(1, Ordering::SeqCst) == 0 {
                        return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({})));
                    }
                    (
                        StatusCode::OK,
                        Json(serde_json::json!({"name":name,"releases":[]})),
                    )
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move { axum::serve(listener, server).await });
        let cache = Cache::new(TTL, 70);
        let client = reqwest::Client::new();
        let first = format!("http://{address}/first");
        assert!(cache.get(&client, "first", &first, false).await.is_err());
        cache
            .get(&client, "first", &first, false)
            .await
            .map_err(|e| e.message)?;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        for name in ["second", "third", "fourth"] {
            cache
                .get(&client, name, &format!("http://{address}/{name}"), false)
                .await
                .map_err(|e| e.message)?;
        }
        assert!(
            cache
                .entries
                .lock()
                .await
                .values()
                .map(|entry| entry.bytes)
                .sum::<usize>()
                <= 70
        );
        cache
            .get(&client, "first", &first, false)
            .await
            .map_err(|e| e.message)?;
        assert_eq!(calls.load(Ordering::SeqCst), 6);
        server.abort();
        Ok(())
    }
}
