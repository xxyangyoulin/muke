use anyhow::{Result, bail};
use curl::easy::{Easy, List, WriteError};
use std::{net::IpAddr, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
use url::Url;

#[derive(Clone)]
pub struct Client {
    slots: Arc<tokio::sync::Semaphore>,
    proxy: Option<String>,
    #[cfg(test)]
    pub(crate) fixture: Option<Arc<std::sync::Mutex<TestResponses>>>,
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct TestResponses {
    pub responses: std::collections::HashMap<String, Vec<u8>>,
    pub statuses: std::collections::HashMap<String, u16>,
    pub requests: Vec<(String, Option<String>)>,
}

impl Client {
    pub fn new(proxy: Option<String>) -> Self {
        Self {
            slots: Arc::new(tokio::sync::Semaphore::new(4)),
            proxy,
            #[cfg(test)]
            fixture: None,
        }
    }

    pub async fn get(
        &self,
        address: &str,
        allow_private: bool,
        limit: usize,
        stop: &CancellationToken,
    ) -> Result<Vec<u8>> {
        self.get_with_referer(address, allow_private, limit, stop, None)
            .await
    }

    /// 与 [Client::get] 相同，但保留非 2xx 响应，返回状态码与正文。
    pub async fn get_status(
        &self,
        address: &str,
        allow_private: bool,
        limit: usize,
        stop: &CancellationToken,
    ) -> Result<(u16, Vec<u8>)> {
        self.send(address, allow_private, limit, stop, None).await
    }

    pub async fn get_with_referer(
        &self,
        address: &str,
        allow_private: bool,
        limit: usize,
        stop: &CancellationToken,
        referer: Option<&str>,
    ) -> Result<Vec<u8>> {
        let (status, body) = self
            .send(address, allow_private, limit, stop, referer)
            .await?;
        if !(200..300).contains(&status) {
            bail!("远程请求失败（HTTP {status}）");
        }
        Ok(body)
    }

    /// 通用请求：支持自定义方法、请求头、正文、Referer 与超时，返回状态码与响应正文。
    #[allow(clippy::too_many_arguments)]
    pub async fn request(
        &self,
        address: &str,
        method: &str,
        headers: &[(String, String)],
        body: Option<Vec<u8>>,
        referer: Option<&str>,
        timeout: Duration,
        allow_private: bool,
        limit: usize,
        stop: &CancellationToken,
    ) -> Result<(u16, Vec<u8>)> {
        #[cfg(test)]
        if let Some(fixture) = &self.fixture {
            if stop.is_cancelled() {
                bail!("任务已取消");
            }
            let mut fixture = fixture.lock().unwrap();
            let recorded = referer
                .map(str::to_owned)
                .or_else(|| (method != "GET").then(|| method.to_owned()));
            fixture.requests.push((address.into(), recorded));
            let response = fixture
                .responses
                .get(address)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("夹具请求失败"))?;
            return Ok((
                fixture.statuses.get(address).copied().unwrap_or(200),
                response,
            ));
        }
        let permit = tokio::select! {
            _ = stop.cancelled() => bail!("任务已取消"),
            permit = self.slots.clone().acquire_owned() => permit?,
        };
        let permit = Arc::new(permit);
        let mut url = parse_url(address)?;
        let headers = headers.to_vec();
        let method = method.to_owned();
        let mut referer = referer.map(str::to_owned);
        for _redirect in 0..=5 {
            let host = url.host_str().unwrap().trim_matches(['[', ']']).to_owned();
            let port = url.port_or_known_default().unwrap();
            let addresses = tokio::select! {
                _ = stop.cancelled() => bail!("任务已取消"),
                result = tokio::time::timeout(Duration::from_secs(10), tokio::net::lookup_host((host.as_str(), port))) => result??.collect::<Vec<_>>(),
            };
            if addresses.is_empty()
                || (!allow_private && addresses.iter().any(|address| !public_ip(address.ip())))
            {
                bail!("目标地址为内网、保留地址或无法解析");
            }
            let ip = addresses[0].ip();
            let proxy = self.proxy.clone();
            let target = url.clone();
            let lease = permit.clone();
            let stop = stop.clone();
            let request_headers = headers.clone();
            let method = method.clone();
            let body = body.clone();
            let request_referer = referer.clone();
            // libcurl resolves CONNECT targets itself unless connect_to pins the validated IP.
            let (status, location, response) = tokio::task::spawn_blocking(move || {
                let _lease = lease;
                let mut request = Easy::new();
                request.url(target.as_str())?;
                request.proxy(proxy.as_deref().unwrap_or(""))?;
                request.follow_location(false)?;
                request.timeout(timeout)?;
                request.connect_timeout(Duration::from_secs(10))?;
                request.useragent("MUKE/0.1")?;
                if let Some(referer) = request_referer {
                    request.referer(&referer)?;
                }
                if method != "GET" {
                    request.custom_request(&method)?;
                }
                if !request_headers.is_empty() {
                    let mut list = List::new();
                    for (name, value) in &request_headers {
                        list.append(&format!("{name}: {value}"))?;
                    }
                    request.http_headers(list)?;
                }
                if let Some(body) = body {
                    request.post_fields_copy(&body)?;
                }
                request.accept_encoding("")?;
                request.progress(true)?;
                let address = match ip {
                    IpAddr::V4(ip) => ip.to_string(),
                    IpAddr::V6(ip) => format!("[{ip}]"),
                };
                let resolve_host = if host.contains(':') {
                    format!("[{host}]")
                } else {
                    host
                };
                let mut resolve = List::new();
                resolve.append(&format!("{resolve_host}:{port}:{address}"))?;
                request.resolve(resolve)?;
                if proxy.as_ref().is_some_and(|s| !s.is_empty()) {
                    let mut targets = List::new();
                    targets.append(&format!("{resolve_host}:{port}:{address}:{port}"))?;
                    request.connect_to(targets)?;
                    request.http_proxy_tunnel(true)?;
                }
                let mut body = Vec::new();
                let mut location = None;
                let mut oversized = false;
                let result = {
                    let mut transfer = request.transfer();
                    transfer.write_function(|data| {
                        if stop.is_cancelled() {
                            return Ok(0);
                        }
                        if body.len() + data.len() > limit {
                            oversized = true;
                            return Ok(0);
                        }
                        body.extend_from_slice(data);
                        Ok::<_, WriteError>(data.len())
                    })?;
                    transfer.header_function(|line| {
                        if let Ok(line) = std::str::from_utf8(line)
                            && let Some((name, value)) = line.split_once(':')
                            && name.eq_ignore_ascii_case("location")
                        {
                            location = Some(value.trim().to_owned());
                        }
                        true
                    })?;
                    transfer.progress_function(|_, _, _, _| !stop.is_cancelled())?;
                    transfer.perform()
                };
                if stop.is_cancelled() {
                    bail!("任务已取消");
                }
                if oversized {
                    bail!("远程响应超过大小限制");
                }
                if result.is_err() {
                    bail!("远程请求失败或超时");
                }
                Ok::<_, anyhow::Error>((request.response_code()?, location, body))
            })
            .await??;
            if (300..400).contains(&status)
                && let Some(location) = location
            {
                let next = parse_url(url.join(&location)?.as_str())?;
                if next.origin() != url.origin() {
                    if headers.iter().any(|(name, _)| {
                        matches!(
                            name.to_ascii_lowercase().as_str(),
                            "authorization" | "x-api-key"
                        )
                    }) {
                        bail!("带凭据的请求不能重定向到其他站点");
                    }
                    referer = None;
                }
                url = next;
                continue;
            }
            drop(permit);
            return Ok((u16::try_from(status).unwrap_or(0), response));
        }
        bail!("重定向次数过多")
    }

    async fn send(
        &self,
        address: &str,
        allow_private: bool,
        limit: usize,
        stop: &CancellationToken,
        referer: Option<&str>,
    ) -> Result<(u16, Vec<u8>)> {
        self.request(
            address,
            "GET",
            &[],
            None,
            referer,
            Duration::from_secs(20),
            allow_private,
            limit,
            stop,
        )
        .await
    }
}

pub fn parse_url(value: &str) -> Result<Url> {
    let url = Url::parse(value).map_err(|_| anyhow::anyhow!("地址格式无效"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        bail!("仅允许无凭据的 HTTP/HTTPS 地址");
    }
    Ok(url)
}

fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || a == 0
                || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 192 && b == 88 && c == 99)
                || (a == 198 && (b == 18 || b == 19)))
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            // Only global unicast; exclude mapped, documentation and transition ranges.
            segments[0] & 0xe000 == 0x2000
                && !(segments[0] == 0x2001 && (segments[1] < 0x200 || segments[1] == 0xdb8))
                && segments[0] != 0x2002
                && !(segments[0] == 0x3fff && segments[1] < 0x1000)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reject_private_reserved_and_credential_urls() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "100.64.0.1",
            "169.254.169.254",
            "192.0.2.1",
            "198.18.0.1",
            "224.0.0.1",
            "::1",
            "::ffff:8.8.8.8",
            "fc00::1",
            "fe80::1",
            "2001:db8::1",
            "2002:7f00:1::",
        ] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111"] {
            assert!(public_ip(ip.parse().unwrap()));
        }
        for url in [
            "file:///etc/passwd",
            "http://user:pass@example.org/",
            "ftp://example.org/",
        ] {
            assert!(parse_url(url).is_err());
        }
    }
}
