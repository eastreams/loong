/// Shared HTTP utilities for outbound HTTP surfaces.
use std::collections::BTreeSet;
use std::sync::Arc;

#[derive(Debug, Clone, Copy)]
pub(crate) struct HttpTargetValidationOptions<'a> {
    pub allow_private_hosts: bool,
    pub reject_userinfo: bool,
    pub resolve_dns: bool,
    pub enforce_allowed_domains: bool,
    pub allowed_domains: Option<&'a BTreeSet<String>>,
    pub blocked_domains: Option<&'a BTreeSet<String>>,
}

pub(crate) fn validate_http_target(
    url: &reqwest::Url,
    options: &HttpTargetValidationOptions<'_>,
    surface_name: &str,
) -> Result<String, String> {
    use std::net::{IpAddr, ToSocketAddrs};

    let scheme = url.scheme();
    let is_http = scheme == "http";
    let is_https = scheme == "https";
    if !is_http && !is_https {
        return Err(format!(
            "{surface_name} requires http or https url, got scheme `{scheme}`"
        ));
    }

    let username = url.username();
    let password = url.password();
    let has_userinfo = !username.is_empty() || password.is_some();
    if options.reject_userinfo && has_userinfo {
        return Err(format!(
            "{surface_name} must not embed credentials in the url userinfo section"
        ));
    }

    let host = url
        .host_str()
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| format!("{surface_name} url has no host"))?;

    let blocked_rule = options
        .blocked_domains
        .and_then(|rules| first_matching_domain_rule(host.as_str(), rules));
    if let Some(rule) = blocked_rule {
        return Err(format!(
            "{surface_name} blocked host `{host}` because it matches blocked domain rule `{rule}`"
        ));
    }

    let allowed_rule = options
        .allowed_domains
        .and_then(|rules| first_matching_domain_rule(host.as_str(), rules));
    if options.enforce_allowed_domains && allowed_rule.is_none() {
        return Err(format!(
            "{surface_name} denied host `{host}` because it is not in allowed_domains"
        ));
    }

    if options.allow_private_hosts {
        return Ok(host);
    }

    if host == "localhost" {
        return Err(format!(
            "{surface_name} blocked private or special-use host `localhost`"
        ));
    }

    let parsed_ip = host.parse::<IpAddr>();
    if let Ok(ip) = parsed_ip {
        if is_private_or_special_ip(ip) {
            return Err(format!(
                "{surface_name} blocked private or special-use address `{ip}`"
            ));
        }
        return Ok(host);
    }

    if !options.resolve_dns {
        return Ok(host);
    }

    let port = url
        .port_or_known_default()
        .ok_or_else(|| format!("{surface_name} url has no known port"))?;
    let addrs = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|error| format!("{surface_name} failed to resolve host `{host}`: {error}"))?;

    let mut saw_addr = false;
    for addr in addrs {
        saw_addr = true;
        if is_private_or_special_ip(addr.ip()) {
            return Err(format!(
                "{surface_name} blocked private or special-use address `{}` for host `{host}`",
                addr.ip()
            ));
        }
    }

    if !saw_addr {
        return Err(format!(
            "{surface_name} resolved no addresses for host `{host}`"
        ));
    }

    Ok(host)
}

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
pub struct SsrfSafeResolver {
    pub allow_private_hosts: bool,
}

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
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
        .map_err(|error| format!("failed to build SSRF-safe HTTP client: {error}"))
}

pub(crate) fn is_private_or_special_ip(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;

    match ip {
        IpAddr::V4(ipv4) => is_private_or_special_ipv4(ipv4),
        IpAddr::V6(ipv6) => is_private_or_special_ipv6(ipv6),
    }
}

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
        || ((segments[0] & 0xffc0) == 0xfec0)
        || (segments[0] == 0x2001 && segments[1] == 0x0db8)
}

fn first_matching_domain_rule<'a>(host: &str, rules: &'a BTreeSet<String>) -> Option<&'a str> {
    rules
        .iter()
        .find(|rule| domain_rule_matches(host, rule.as_str()))
        .map(String::as_str)
}

fn domain_rule_matches(host: &str, rule: &str) -> bool {
    if let Some(suffix) = rule.strip_prefix("*.") {
        let suffix_match = host == suffix;
        let subdomain_match = host.ends_with(format!(".{suffix}").as_str());
        return suffix_match || subdomain_match;
    }

    host == rule
}

#[cfg(all(test, any(feature = "tool-webfetch", feature = "tool-websearch")))]
#[allow(clippy::panic)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::time::Duration;

    fn must<T, E>(result: Result<T, E>, context: &str) -> T
    where
        E: std::fmt::Display,
    {
        match result {
            Ok(value) => value,
            Err(error) => panic!("{context}: {error}"),
        }
    }

    fn spawn_http_server() -> Result<
        (
            String,
            mpsc::Receiver<String>,
            std::thread::JoinHandle<Result<bool, String>>,
        ),
        String,
    > {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|error| format!("bind test server: {error}"))?;
        let address = listener
            .local_addr()
            .map_err(|error| format!("resolve test server address: {error}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|error| format!("configure test server nonblocking mode: {error}"))?;
        let (request_tx, request_rx) = mpsc::sync_channel(1);

        let handle = std::thread::spawn(move || -> Result<bool, String> {
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            let (mut stream, _peer) = loop {
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if std::time::Instant::now() >= deadline {
                            return Ok(false);
                        }
                        std::thread::park_timeout(Duration::from_millis(20));
                    }
                    Err(error) => return Err(format!("accept test client: {error}")),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .map_err(|error| format!("set read timeout: {error}"))?;

            let mut buffer = [0u8; 4096];
            let byte_count = stream
                .read(&mut buffer)
                .map_err(|error| format!("read request bytes: {error}"))?;
            let request_bytes = buffer
                .get(..byte_count)
                .ok_or_else(|| format!("captured request length out of range: {byte_count}"))?;
            request_tx
                .send(String::from_utf8_lossy(request_bytes).into_owned())
                .map_err(|error| format!("send captured request: {error}"))?;

            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nok",
                )
                .map_err(|error| format!("write response: {error}"))?;
            stream
                .flush()
                .map_err(|error| format!("flush response: {error}"))?;
            Ok(true)
        });

        Ok((
            format!("http://localhost:{}", address.port()),
            request_rx,
            handle,
        ))
    }

    #[test]
    fn build_ssrf_safe_client_allows_localhost_when_private_hosts_are_enabled() {
        let (url, request_rx, server_handle) = must(spawn_http_server(), "spawn http server");
        let user_agent = "LoongClaw-WebHttp-Test/1.0";
        let client = must(build_ssrf_safe_client(true, 5, user_agent), "build client");

        let response = must(
            run_async(async {
                client
                    .get(url)
                    .send()
                    .await
                    .map_err(|error| error.to_string())?
                    .text()
                    .await
                    .map_err(|error| error.to_string())
            }),
            "run async request",
        );
        let body = must(response, "request should succeed");

        assert_eq!(body, "ok");

        let request = must(
            request_rx
                .recv_timeout(Duration::from_secs(5))
                .map_err(|error| format!("capture request: {error}")),
            "capture request",
        );
        assert!(
            request
                .to_ascii_lowercase()
                .contains(&format!("user-agent: {}", user_agent.to_ascii_lowercase())),
            "expected user-agent header in request: {request}"
        );

        let accepted_request = match server_handle.join() {
            Ok(result) => result,
            Err(_panic) => panic!("join test server: thread panicked"),
        };
        assert!(
            must(accepted_request, "test server exited with error"),
            "expected localhost test server to receive the request"
        );
    }

    #[test]
    fn build_ssrf_safe_client_blocks_localhost_when_private_hosts_are_disabled() {
        let (url, _request_rx, server_handle) = must(spawn_http_server(), "spawn http server");
        let client = must(
            build_ssrf_safe_client(false, 5, "LoongClaw-WebHttp-Test/1.0"),
            "build client",
        );

        let response = must(
            run_async(async {
                client
                    .get(url)
                    .send()
                    .await
                    .map_err(|error| error.to_string())
            }),
            "run async request",
        );
        let error = match response {
            Ok(_response) => panic!("localhost should be blocked when private hosts are disabled"),
            Err(error) => error,
        };
        let accepted_request = match server_handle.join() {
            Ok(result) => result,
            Err(_panic) => panic!("join test server: thread panicked"),
        };

        assert!(
            !error.is_empty(),
            "expected a request error when private hosts are disabled"
        );
        assert!(
            !must(accepted_request, "test server exited with error"),
            "unexpected error: {error}"
        );
    }
}
