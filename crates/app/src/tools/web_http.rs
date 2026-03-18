/// Shared HTTP utilities for web tools (web.fetch, web.search).
/// Provides SSRF-safe DNS resolution and other common patterns.
use std::sync::Arc;

/// Bridge sync-to-async execution for web tools.
///
/// Cases handled:
/// - Multi-thread runtime: use `block_in_place` + `block_on`
/// - Current-thread runtime: run future on a dedicated worker thread
/// - No runtime: create a temporary current-thread runtime
#[cfg(any(feature = "tool-webfetch", feature = "tool-websearch"))]
pub fn run_async<F>(fut: F) -> Result<F::Output, String>
where
    F: std::future::Future + Send,
    F::Output: Send,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            Ok(tokio::task::block_in_place(|| handle.block_on(fut)))
        }
        Ok(_) => std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("failed to create tokio runtime for web tools: {error}")
                        })?;
                    Ok(rt.block_on(fut))
                })
                .join()
                .map_err(|_panic| "web tools async worker thread panicked".to_owned())?
        }),
        Err(_) => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| {
                    format!("failed to create tokio runtime for web tools: {error}")
                })?;
            Ok(rt.block_on(fut))
        }
    }
}

/// Custom DNS resolver that rejects private/special-use IP addresses at
/// connection time, eliminating the TOCTOU window between validation and
/// the HTTP client's own DNS resolution.
#[cfg(any(feature = "tool-webfetch", feature = "tool-websearch"))]
pub struct SsrfSafeResolver {
    pub allow_private_hosts: bool,
}

#[cfg(any(feature = "tool-webfetch", feature = "tool-websearch"))]
impl reqwest::dns::Resolve for SsrfSafeResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let allow_private = self.allow_private_hosts;
        Box::pin(async move {
            let host = name.as_str();
            let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host, 0))
                .await
                .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })?
                .collect();

            if addrs.is_empty() {
                return Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("web HTTP resolved no addresses for host `{host}`"),
                ))
                    as Box<dyn std::error::Error + Send + Sync>);
            }

            if !allow_private {
                for addr in &addrs {
                    if is_private_or_special_ip(addr.ip()) {
                        return Err(Box::new(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            format!(
                                "web HTTP blocked private or special-use address `{}` for host `{host}`",
                                addr.ip()
                            ),
                        ))
                            as Box<dyn std::error::Error + Send + Sync>);
                    }
                }
            }

            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// Build an SSRF-safe HTTP client for web tools.
#[cfg(any(feature = "tool-webfetch", feature = "tool-websearch"))]
pub fn build_ssrf_safe_client(
    allow_private_hosts: bool,
    timeout_seconds: u64,
    user_agent: &str,
) -> Result<reqwest::Client, String> {
    let resolver = SsrfSafeResolver {
        allow_private_hosts,
    };
    reqwest::Client::builder()
        .dns_resolver(Arc::new(resolver))
        .timeout(std::time::Duration::from_secs(timeout_seconds))
        .user_agent(user_agent)
        .build()
        .map_err(|error| format!("failed to build SSRF-safe HTTP client: {error}"))
}

#[cfg(any(
    feature = "tool-webfetch",
    feature = "tool-browser",
    feature = "tool-websearch"
))]
pub(crate) fn is_private_or_special_ip(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;

    match ip {
        IpAddr::V4(ipv4) => is_private_or_special_ipv4(ipv4),
        IpAddr::V6(ipv6) => is_private_or_special_ipv6(ipv6),
    }
}

#[cfg(any(
    feature = "tool-webfetch",
    feature = "tool-browser",
    feature = "tool-websearch"
))]
pub(crate) fn is_private_or_special_ipv4(ip: std::net::Ipv4Addr) -> bool {
    let octets = ip.octets();
    let first = octets[0];
    let second = octets[1];
    let third = octets[2];

    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_unspecified()
        || ip.is_multicast()
        || first == 0
        || (first == 100 && (64..=127).contains(&second))
        || (first == 192 && second == 0 && third == 0)
        || (first == 198 && matches!(second, 18 | 19))
        || (first == 198 && second == 51 && third == 100)
        || (first == 203 && second == 0 && third == 113)
        || first >= 240
}

#[cfg(any(
    feature = "tool-webfetch",
    feature = "tool-browser",
    feature = "tool-websearch"
))]
pub(crate) fn is_private_or_special_ipv6(ip: std::net::Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return true;
    }

    // Check both IPv4-mapped (::ffff:x.x.x.x) and IPv4-compatible (::x.x.x.x)
    // addresses. IPv4-compatible addresses are deprecated (RFC 4291) but still
    // parseable, and we must not allow them to bypass the private-IP filter.
    if let Some(ipv4) = ip.to_ipv4_mapped().or_else(|| ip.to_ipv4()) {
        return is_private_or_special_ipv4(ipv4);
    }

    let segments = ip.segments();
    ((segments[0] & 0xfe00) == 0xfc00)
        || ((segments[0] & 0xffc0) == 0xfe80)
        || (segments[0] == 0x2001 && segments[1] == 0x0db8)
}
