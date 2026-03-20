use std::net::IpAddr;

use loongclaw_app as mvp;

#[cfg(not(test))]
use std::collections::{BTreeMap, BTreeSet};
#[cfg(not(test))]
use std::net::SocketAddr;
#[cfg(not(test))]
use std::time::Duration;
#[cfg(not(test))]
use tokio::net::{TcpStream, lookup_host};
#[cfg(not(test))]
use tokio::time::timeout;

pub(crate) const PROVIDER_ROUTE_PROBE_CHECK_NAME: &str = "provider route probe";
pub(crate) const MODEL_CATALOG_TRANSPORT_FAILED_MARKER: &str = "model catalog transport failed";

#[cfg(not(test))]
const ROUTE_PROBE_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(not(test))]
const ROUTE_PROBE_CONNECT_ADDRESS_LIMIT: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderRouteProbeLevel {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderRouteProbe {
    pub(crate) level: ProviderRouteProbeLevel,
    pub(crate) detail: String,
}

#[cfg(not(test))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProviderRouteTarget {
    label: String,
    host: String,
    port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProviderRouteObservation {
    label: String,
    host: String,
    port: u16,
    resolved_addrs: Result<Vec<IpAddr>, String>,
    connect_result: Option<Result<IpAddr, String>>,
}

pub(crate) fn is_transport_style_model_probe_failure(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    let looks_like_model_probe_transport = lower.contains("model-list request failed on attempt")
        || lower.contains("model-list request setup failed on attempt");
    let looks_like_route_issue = lower.contains("dns")
        || lower.contains("lookup address")
        || lower.contains("name or service not known")
        || lower.contains("nodename nor servname")
        || lower.contains("temporary failure in name resolution")
        || lower.contains("failed to lookup address information")
        || lower.contains("no such host")
        || lower.contains("timed out")
        || lower.contains("proxy")
        || lower.contains("tunnel")
        || lower.contains("tun")
        || lower.contains("connect")
        || lower.contains("connection");
    looks_like_model_probe_transport && looks_like_route_issue
}

#[cfg(test)]
pub(crate) async fn collect_provider_route_probe(
    provider: &mvp::config::ProviderConfig,
) -> Option<ProviderRouteProbe> {
    let _ = provider;
    None
}

#[cfg(not(test))]
pub(crate) async fn collect_provider_route_probe(
    provider: &mvp::config::ProviderConfig,
) -> Option<ProviderRouteProbe> {
    let targets = provider_route_targets(provider);
    if targets.is_empty() {
        return None;
    }

    let mut observations = Vec::with_capacity(targets.len());
    for target in &targets {
        observations.push(probe_route_target(target).await);
    }

    Some(summarize_provider_route_probe(&observations))
}

#[cfg(not(test))]
fn provider_route_targets(provider: &mvp::config::ProviderConfig) -> Vec<ProviderRouteTarget> {
    let mut grouped = BTreeMap::<(String, u16), Vec<&'static str>>::new();
    push_provider_route_target(&mut grouped, "request", provider.endpoint().as_str());
    push_provider_route_target(&mut grouped, "models", provider.models_endpoint().as_str());

    grouped
        .into_iter()
        .map(|((host, port), labels)| ProviderRouteTarget {
            label: labels.join("/"),
            host,
            port,
        })
        .collect()
}

#[cfg(not(test))]
fn push_provider_route_target(
    grouped: &mut BTreeMap<(String, u16), Vec<&'static str>>,
    label: &'static str,
    url: &str,
) {
    let Some(target) = parse_provider_route_target(url) else {
        return;
    };
    let entry = grouped.entry((target.host, target.port)).or_default();
    if !entry.contains(&label) {
        entry.push(label);
    }
}

#[cfg(not(test))]
fn parse_provider_route_target(url: &str) -> Option<ProviderRouteTarget> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let host = parsed.host_str()?.to_owned();
    let port = parsed.port_or_known_default()?;
    Some(ProviderRouteTarget {
        label: "request".to_owned(),
        host,
        port,
    })
}

#[cfg(not(test))]
async fn probe_route_target(target: &ProviderRouteTarget) -> ProviderRouteObservation {
    let resolved_addrs = lookup_host((target.host.as_str(), target.port))
        .await
        .map(|addrs| {
            let mut unique = BTreeSet::new();
            for addr in addrs {
                unique.insert(addr.ip());
            }
            unique.into_iter().collect::<Vec<_>>()
        })
        .map_err(|error| error.to_string());
    let connect_result = match &resolved_addrs {
        Ok(addrs) => probe_route_connectivity(addrs, target.port).await,
        Err(_) => None,
    };

    ProviderRouteObservation {
        label: target.label.clone(),
        host: target.host.clone(),
        port: target.port,
        resolved_addrs,
        connect_result,
    }
}

#[cfg(not(test))]
async fn probe_route_connectivity(addrs: &[IpAddr], port: u16) -> Option<Result<IpAddr, String>> {
    if addrs.is_empty() {
        return None;
    }

    let mut last_error = None;
    for addr in addrs.iter().take(ROUTE_PROBE_CONNECT_ADDRESS_LIMIT) {
        let socket = SocketAddr::new(*addr, port);
        match timeout(ROUTE_PROBE_CONNECT_TIMEOUT, TcpStream::connect(socket)).await {
            Ok(Ok(stream)) => {
                drop(stream);
                return Some(Ok(*addr));
            }
            Ok(Err(error)) => last_error = Some(error.to_string()),
            Err(_) => {
                last_error = Some(format!(
                    "timed out after {}s",
                    ROUTE_PROBE_CONNECT_TIMEOUT.as_secs()
                ));
            }
        }
    }

    Some(Err(
        last_error.unwrap_or_else(|| "connection failed".to_owned())
    ))
}

fn summarize_provider_route_probe(observations: &[ProviderRouteObservation]) -> ProviderRouteProbe {
    let mut level = ProviderRouteProbeLevel::Pass;
    let mut details = Vec::with_capacity(observations.len());

    for observation in observations {
        let (observation_level, detail) = summarize_route_observation(observation);
        level = combine_probe_levels(level, observation_level);
        details.push(detail);
    }

    ProviderRouteProbe {
        level,
        detail: details.join(" "),
    }
}

fn summarize_route_observation(
    observation: &ProviderRouteObservation,
) -> (ProviderRouteProbeLevel, String) {
    let target = format!(
        "{} host {}:{}",
        observation.label, observation.host, observation.port
    );
    match &observation.resolved_addrs {
        Err(error) => (
            ProviderRouteProbeLevel::Fail,
            format!(
                "{target}: dns lookup failed ({error}). check local dns or proxy/TUN rules before retrying."
            ),
        ),
        Ok(addrs) if addrs.is_empty() => (
            ProviderRouteProbeLevel::Fail,
            format!(
                "{target}: dns lookup returned no addresses. check local dns or proxy/TUN rules before retrying."
            ),
        ),
        Ok(addrs) => {
            let rendered_addrs = render_probe_addresses(addrs);
            let fake_ip_style = addrs.iter().any(is_likely_fake_ip_addr);
            match observation.connect_result.as_ref() {
                Some(Ok(connected_ip)) if fake_ip_style => (
                    ProviderRouteProbeLevel::Warn,
                    format!(
                        "{target}: dns resolved to {rendered_addrs} (fake-ip-style); tcp connect ok via {connected_ip}. the route currently depends on local fake-ip/TUN interception, so intermittent long-request failures usually point to proxy health or direct/bypass rules."
                    ),
                ),
                Some(Ok(connected_ip)) => (
                    ProviderRouteProbeLevel::Pass,
                    format!(
                        "{target}: dns resolved to {rendered_addrs}; tcp connect ok via {connected_ip}. basic route reachability looks healthy right now, so the earlier transport failure is more likely upstream or transient proxy instability."
                    ),
                ),
                Some(Err(error)) if fake_ip_style => (
                    ProviderRouteProbeLevel::Fail,
                    format!(
                        "{target}: dns resolved to {rendered_addrs} (fake-ip-style); tcp connect failed ({error}). the fake-ip/TUN route is not healthy enough to reach the provider right now."
                    ),
                ),
                Some(Err(error)) => (
                    ProviderRouteProbeLevel::Fail,
                    format!(
                        "{target}: dns resolved to {rendered_addrs}; tcp connect failed ({error}). the provider host is not reachable from the current route."
                    ),
                ),
                None => (
                    ProviderRouteProbeLevel::Fail,
                    format!(
                        "{target}: dns resolved to {rendered_addrs}, but no tcp connectivity probe could be completed."
                    ),
                ),
            }
        }
    }
}

fn combine_probe_levels(
    left: ProviderRouteProbeLevel,
    right: ProviderRouteProbeLevel,
) -> ProviderRouteProbeLevel {
    match (left, right) {
        (ProviderRouteProbeLevel::Fail, _) | (_, ProviderRouteProbeLevel::Fail) => {
            ProviderRouteProbeLevel::Fail
        }
        (ProviderRouteProbeLevel::Warn, _) | (_, ProviderRouteProbeLevel::Warn) => {
            ProviderRouteProbeLevel::Warn
        }
        _ => ProviderRouteProbeLevel::Pass,
    }
}

fn render_probe_addresses(addrs: &[IpAddr]) -> String {
    let rendered = addrs
        .iter()
        .take(4)
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if addrs.len() > rendered.len() {
        format!(
            "{} (+{} more)",
            rendered.join(", "),
            addrs.len() - rendered.len()
        )
    } else {
        rendered.join(", ")
    }
}

fn is_likely_fake_ip_addr(addr: &IpAddr) -> bool {
    match addr {
        // 198.18.0.0/15 is the RFC 2544 benchmark range and is commonly reused by fake-ip resolvers.
        IpAddr::V4(ipv4) => {
            let [first, second, _, _] = ipv4.octets();
            first == 198 && (second == 18 || second == 19)
        }
        IpAddr::V6(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn transport_style_model_probe_failure_detection_ignores_status_errors() {
        assert!(!is_transport_style_model_probe_failure(
            "provider returned status 401 on attempt 3/3"
        ));
        assert!(is_transport_style_model_probe_failure(
            "provider model-list request failed on attempt 3/3: operation timed out"
        ));
    }

    #[test]
    fn summarize_provider_route_probe_reports_dns_failures() {
        let probe = summarize_provider_route_probe(&[ProviderRouteObservation {
            label: "request/models".to_owned(),
            host: "api.openai.com".to_owned(),
            port: 443,
            resolved_addrs: Err("no such host".to_owned()),
            connect_result: None,
        }]);

        assert_eq!(probe.level, ProviderRouteProbeLevel::Fail);
        assert!(probe.detail.contains("dns lookup failed"));
        assert!(probe.detail.contains("proxy/TUN"));
    }

    #[test]
    fn summarize_provider_route_probe_warns_for_fake_ip_style_dns() {
        let probe = summarize_provider_route_probe(&[ProviderRouteObservation {
            label: "request/models".to_owned(),
            host: "ark.cn-beijing.volces.com".to_owned(),
            port: 443,
            resolved_addrs: Ok(vec![IpAddr::V4(Ipv4Addr::new(198, 18, 0, 2))]),
            connect_result: Some(Ok(IpAddr::V4(Ipv4Addr::new(198, 18, 0, 2)))),
        }]);

        assert_eq!(probe.level, ProviderRouteProbeLevel::Warn);
        assert!(probe.detail.contains("fake-ip-style"));
        assert!(probe.detail.contains("direct/bypass"));
    }

    #[test]
    fn summarize_provider_route_probe_reports_connect_failures() {
        let probe = summarize_provider_route_probe(&[ProviderRouteObservation {
            label: "request".to_owned(),
            host: "api.openai.com".to_owned(),
            port: 443,
            resolved_addrs: Ok(vec![IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))]),
            connect_result: Some(Err("timed out after 2s".to_owned())),
        }]);

        assert_eq!(probe.level, ProviderRouteProbeLevel::Fail);
        assert!(probe.detail.contains("tcp connect failed"));
        assert!(probe.detail.contains("not reachable"));
    }
}
