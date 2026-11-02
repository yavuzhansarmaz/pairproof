//! Automatic direct-connection setup: STUN external-address discovery
//! plus NAT-PMP / UPnP IGD port mapping. Every probe is best-effort and
//! strictly bounded; any failure yields `None` and the caller falls back
//! to the loopback endpoint. No user action is ever required.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

use rand::Rng;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::time::timeout;

use crate::connection::Endpoint;

use super::preflight::{classify_ipv4, primary_local_ipv4, IpClass};

const STUN_SERVERS: [(&str, u16); 2] =
    [("stun.l.google.com", 19302), ("stun.cloudflare.com", 3478)];
const STUN_COOKIE: u32 = 0x2112_A442;
const NATPMP_PORT: u16 = 5351;
const NATPMP_LIFETIME_SECS: u32 = 7200;
const PROBE_TIMEOUT: Duration = Duration::from_millis(1500);
const MAX_HTTP_BYTES: usize = 64 * 1024;

/// A reachable direct endpoint plus the mapping that keeps it reachable.
#[derive(Debug)]
pub struct DirectAccess {
    endpoint: Endpoint,
    mapping: Option<PortMapping>,
}

impl DirectAccess {
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Splits into the endpoint and the mapping handle (for cleanup).
    pub fn into_parts(self) -> (Endpoint, Option<PortMapping>) {
        (self.endpoint, self.mapping)
    }
}

/// One live router port mapping. `close` removes it; dropping without
/// closing leaves at most a stale lease (NAT-PMP expires on its own,
/// UPnP is deleted at session end on the production path).
#[derive(Debug)]
pub enum PortMapping {
    NatPmp {
        gateway: Ipv4Addr,
        internal_port: u16,
        external_port: u16,
    },
    Upnp {
        control_url: String,
        service_type: String,
        external_port: u16,
    },
}

impl PortMapping {
    /// Best-effort removal of the mapping from the router.
    pub async fn close(&self) {
        match self {
            PortMapping::NatPmp {
                gateway,
                internal_port,
                external_port,
            } => {
                let _ = natpmp_request(
                    *gateway,
                    &build_natpmp_mapping_request(*internal_port, *external_port, 0),
                )
                .await;
            }
            PortMapping::Upnp {
                control_url,
                service_type,
                external_port,
            } => {
                let _ = upnp_delete_mapping(control_url, service_type, *external_port).await;
            }
        }
    }
}

/// Tries, in order: public local address, NAT-PMP, UPnP IGD. Returns
/// `None` when no automatic path exists (for example behind CGNAT).
pub async fn establish_direct_access(internal_port: u16) -> Option<DirectAccess> {
    let local = primary_local_ipv4()?;
    if classify_ipv4(local) == IpClass::Public {
        let endpoint = Endpoint::public(&local.to_string(), internal_port).ok()?;
        return Some(DirectAccess {
            endpoint,
            mapping: None,
        });
    }
    match natpmp_map(local, internal_port).await {
        NatpmpOutcome::Mapped(access) => return Some(access),
        // The router's own WAN address is not public: a second NAT layer
        // (CGNAT or double NAT) sits upstream, so UPnP on this router
        // cannot help either. Do not pretend otherwise.
        NatpmpOutcome::BehindSharedNat => return None,
        NatpmpOutcome::Unavailable => {}
    }
    upnp_map(local, internal_port).await
}

/// Discovers the external IPv4 as seen from the internet via STUN.
pub async fn discover_external_ip() -> Option<Ipv4Addr> {
    for (host, port) in STUN_SERVERS {
        if let Some(ip) = stun_mapped_address(host, port).await {
            if !matches!(classify_ipv4(ip), IpClass::Loopback | IpClass::Unspecified) {
                return Some(ip);
            }
        }
    }
    None
}

async fn stun_mapped_address(host: &str, port: u16) -> Option<Ipv4Addr> {
    let addr = tokio::net::lookup_host((host, port))
        .await
        .ok()?
        .find(|a| a.is_ipv4())?;
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await.ok()?;
    socket.connect(addr).await.ok()?;
    let txn: [u8; 12] = rand::thread_rng().gen();
    socket.send(&build_stun_binding_request(txn)).await.ok()?;
    let mut buf = [0u8; 512];
    let n = timeout(PROBE_TIMEOUT, socket.recv(&mut buf))
        .await
        .ok()?
        .ok()?;
    parse_stun_binding_response(&buf[..n], &txn).map(|(ip, _)| ip)
}

/// Builds a 20-byte STUN binding request for `txn`.
pub fn build_stun_binding_request(txn: [u8; 12]) -> [u8; 20] {
    let mut req = [0u8; 20];
    req[0..2].copy_from_slice(&0x0001u16.to_be_bytes());
    req[4..8].copy_from_slice(&STUN_COOKIE.to_be_bytes());
    req[8..20].copy_from_slice(&txn);
    req
}

/// Parses a STUN binding response, validating cookie and transaction id.
/// Returns the mapped `(address, port)` from XOR-MAPPED-ADDRESS or
/// MAPPED-ADDRESS.
pub fn parse_stun_binding_response(data: &[u8], txn: &[u8; 12]) -> Option<(Ipv4Addr, u16)> {
    if data.len() < 20 || u16::from_be_bytes([data[0], data[1]]) != 0x0101 {
        return None;
    }
    if u32::from_be_bytes([data[4], data[5], data[6], data[7]]) != STUN_COOKIE {
        return None;
    }
    if &data[8..20] != txn {
        return None;
    }
    let msg_len = u16::from_be_bytes([data[2], data[3]]) as usize;
    let end = (20 + msg_len).min(data.len());
    let mut pos = 20;
    while pos + 4 <= end {
        let kind = u16::from_be_bytes([data[pos], data[pos + 1]]);
        let len = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        let value = data.get(pos + 4..pos + 4 + len)?;
        if (kind == 0x0020 || kind == 0x0001) && len >= 8 && value[1] == 1 {
            let raw_port = u16::from_be_bytes([value[2], value[3]]);
            let raw_ip = u32::from_be_bytes([value[4], value[5], value[6], value[7]]);
            let (ip, port) = if kind == 0x0020 {
                (
                    Ipv4Addr::from(raw_ip ^ STUN_COOKIE),
                    raw_port ^ (STUN_COOKIE >> 16) as u16,
                )
            } else {
                (Ipv4Addr::from(raw_ip), raw_port)
            };
            return Some((ip, port));
        }
        pos += 4 + ((len + 3) & !3);
    }
    None
}

/// Builds the NAT-PMP external-address request.
pub fn build_natpmp_external_request() -> [u8; 2] {
    [0, 0]
}

/// Builds a NAT-PMP TCP mapping request. Lifetime 0 deletes the mapping.
pub fn build_natpmp_mapping_request(internal: u16, external: u16, lifetime: u32) -> [u8; 12] {
    let mut req = [0u8; 12];
    req[1] = 1;
    req[4..6].copy_from_slice(&internal.to_be_bytes());
    req[6..8].copy_from_slice(&external.to_be_bytes());
    req[8..12].copy_from_slice(&lifetime.to_be_bytes());
    req
}

/// Parses a NAT-PMP external-address response (opcode 128, result 0).
pub fn parse_natpmp_external_response(data: &[u8]) -> Option<Ipv4Addr> {
    if data.len() < 12 || data[0] != 0 || data[1] != 128 {
        return None;
    }
    if u16::from_be_bytes([data[2], data[3]]) != 0 {
        return None;
    }
    Some(Ipv4Addr::new(data[8], data[9], data[10], data[11]))
}

/// Parses a NAT-PMP mapping response (opcode 129, result 0), returning
/// the assigned external port.
pub fn parse_natpmp_mapping_response(data: &[u8]) -> Option<u16> {
    if data.len() < 16 || data[0] != 0 || data[1] != 129 {
        return None;
    }
    if u16::from_be_bytes([data[2], data[3]]) != 0 {
        return None;
    }
    Some(u16::from_be_bytes([data[10], data[11]]))
}

/// Extracts the default gateway from Linux `/proc/net/route` content.
pub fn parse_route_table(text: &str) -> Option<Ipv4Addr> {
    for line in text.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 4 || fields[1] != "00000000" || fields[2] == "00000000" {
            continue;
        }
        if let Ok(flags) = u16::from_str_radix(fields[3], 16) {
            if flags & 0x0002 == 0 {
                continue;
            }
        }
        let raw = u32::from_str_radix(fields[2], 16).ok()?;
        return Some(Ipv4Addr::from(raw.to_le_bytes()));
    }
    None
}

/// Best-effort default gateway: route table on Linux, else the common
/// `.1` address of the local private subnet.
pub fn default_gateway(local: Ipv4Addr) -> Option<Ipv4Addr> {
    #[cfg(target_os = "linux")]
    {
        if let Ok(text) = std::fs::read_to_string("/proc/net/route") {
            if let Some(gateway) = parse_route_table(&text) {
                return Some(gateway);
            }
        }
    }
    let _ = local;
    let octets = local.octets();
    if classify_ipv4(local) == IpClass::Private {
        Some(Ipv4Addr::new(octets[0], octets[1], octets[2], 1))
    } else {
        None
    }
}

enum NatpmpOutcome {
    Mapped(DirectAccess),
    BehindSharedNat,
    Unavailable,
}

async fn natpmp_request(gateway: Ipv4Addr, request: &[u8]) -> Option<Vec<u8>> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await.ok()?;
    socket
        .connect(SocketAddrV4::new(gateway, NATPMP_PORT))
        .await
        .ok()?;
    socket.send(request).await.ok()?;
    let mut buf = [0u8; 64];
    let n = timeout(PROBE_TIMEOUT, socket.recv(&mut buf))
        .await
        .ok()?
        .ok()?;
    Some(buf[..n].to_vec())
}

async fn natpmp_map(local: Ipv4Addr, internal_port: u16) -> NatpmpOutcome {
    let Some(gateway) = default_gateway(local) else {
        return NatpmpOutcome::Unavailable;
    };
    natpmp_map_at(gateway, internal_port).await
}

async fn natpmp_map_at(gateway: Ipv4Addr, internal_port: u16) -> NatpmpOutcome {
    let Some(response) = natpmp_request(gateway, &build_natpmp_external_request()).await else {
        return NatpmpOutcome::Unavailable;
    };
    let Some(external_ip) = parse_natpmp_external_response(&response) else {
        return NatpmpOutcome::Unavailable;
    };
    if classify_ipv4(external_ip) != IpClass::Public {
        return NatpmpOutcome::BehindSharedNat;
    }
    let request = build_natpmp_mapping_request(internal_port, internal_port, NATPMP_LIFETIME_SECS);
    let Some(response) = natpmp_request(gateway, &request).await else {
        return NatpmpOutcome::Unavailable;
    };
    let Some(external_port) = parse_natpmp_mapping_response(&response) else {
        return NatpmpOutcome::Unavailable;
    };
    let Ok(endpoint) = Endpoint::public(&external_ip.to_string(), external_port) else {
        return NatpmpOutcome::Unavailable;
    };
    NatpmpOutcome::Mapped(DirectAccess {
        endpoint,
        mapping: Some(PortMapping::NatPmp {
            gateway,
            internal_port,
            external_port,
        }),
    })
}

/// Extracts the `LOCATION` header value from an SSDP response.
pub fn extract_ssdp_location(response: &str) -> Option<String> {
    for line in response.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("location") {
            let value = value.trim().to_string();
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

/// One UPnP IGD control service resolved to an absolute, same-origin URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgdService {
    pub service_type: String,
    pub control_url: String,
}

/// Picks the WANIPConnection (or WANPPPConnection) service from a device
/// description. Absolute control URLs must share the description origin;
/// anything else is rejected.
pub fn select_igd_service(xml: &str, location: &str) -> Option<IgdService> {
    let origin = parse_http_url(location)?;
    for marker in [
        "urn:schemas-upnp-org:service:WANIPConnection:",
        "urn:schemas-upnp-org:service:WANPPPConnection:",
    ] {
        let Some(start) = xml.find(marker) else {
            continue;
        };
        // The service type runs to the closing tag; keep it verbatim but
        // reject markup-breaking characters before reuse in SOAP.
        let tail = &xml[start..];
        let Some(end) = tail.find("</serviceType>") else {
            continue;
        };
        let service_type = &tail[..end];
        if service_type
            .chars()
            .any(|c| c.is_control() || "<>&\"'".contains(c))
        {
            continue;
        }
        let Some(url_start) = tail.find("<controlURL>") else {
            continue;
        };
        let rest = &tail[url_start + "<controlURL>".len()..];
        let Some(url_end) = rest.find("</controlURL>") else {
            continue;
        };
        let raw = rest[..url_end].trim();
        if raw.is_empty() || raw.chars().any(|c| c.is_whitespace() || c.is_control()) {
            continue;
        }
        let control_url = if let Some(absolute) = parse_http_url(raw) {
            if absolute.host != origin.host || absolute.port != origin.port {
                return None;
            }
            raw.to_string()
        } else if let Some(path) = raw.strip_prefix('/') {
            format!("http://{}:{}/{}", origin.host, origin.port, path)
        } else {
            format!("http://{}:{}/{}", origin.host, origin.port, raw)
        };
        return Some(IgdService {
            service_type: service_type.to_string(),
            control_url,
        });
    }
    None
}

/// Extracts `NewExternalIPAddress` from a SOAP response body.
pub fn parse_external_ip_soap(body: &str) -> Option<Ipv4Addr> {
    let start = body.find("<NewExternalIPAddress>")? + "<NewExternalIPAddress>".len();
    let end = body[start..].find("</NewExternalIPAddress>")? + start;
    body[start..end].trim().parse().ok()
}

#[derive(Clone, PartialEq, Eq)]
struct HttpUrl {
    host: Ipv4Addr,
    port: u16,
    path: String,
}

fn parse_http_url(url: &str) -> Option<HttpUrl> {
    let rest = url.strip_prefix("http://")?;
    let (authority, path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host.parse().ok()?, port.parse().ok()?),
        None => (authority.parse().ok()?, 80),
    };
    Some(HttpUrl {
        host,
        port,
        path: path.to_string(),
    })
}

async fn http_exchange(url: &HttpUrl, request: &str) -> Option<String> {
    let mut stream = timeout(
        PROBE_TIMEOUT,
        TcpStream::connect(SocketAddrV4::new(url.host, url.port)),
    )
    .await
    .ok()?
    .ok()?;
    timeout(PROBE_TIMEOUT, stream.write_all(request.as_bytes()))
        .await
        .ok()?
        .ok()?;
    let mut raw = Vec::new();
    let mut buf = [0u8; 4096];
    while raw.len() < MAX_HTTP_BYTES {
        let n = timeout(PROBE_TIMEOUT, stream.read(&mut buf))
            .await
            .ok()?
            .ok()?;
        if n == 0 {
            break;
        }
        raw.extend_from_slice(&buf[..n]);
    }
    let text = String::from_utf8_lossy(&raw).into_owned();
    let status: u32 = text
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())?;
    if status != 200 {
        return None;
    }
    let body = text
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())?;
    Some(body)
}

async fn upnp_discover_location() -> Option<String> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await.ok()?;
    let search = "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 1\r\nST: urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\n\r\n";
    socket
        .send_to(search.as_bytes(), (Ipv4Addr::new(239, 255, 255, 250), 1900))
        .await
        .ok()?;
    let deadline = tokio::time::Instant::now() + PROBE_TIMEOUT;
    let mut buf = [0u8; 2048];
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        let n = timeout(remaining, socket.recv(&mut buf)).await.ok()?.ok()?;
        let text = String::from_utf8_lossy(&buf[..n]).into_owned();
        if !text.contains("InternetGatewayDevice") {
            continue;
        }
        let Some(location) = extract_ssdp_location(&text) else {
            continue;
        };
        // The gateway must be a LAN address; never follow a location
        // pointing at a public or otherwise unexpected host.
        if let Some(url) = parse_http_url(&location) {
            if matches!(
                classify_ipv4(url.host),
                IpClass::Private | IpClass::LinkLocal
            ) {
                return Some(location);
            }
        }
    }
}

fn soap_envelope(service_type: &str, action: &str, args: &str) -> String {
    format!(
        "<?xml version=\"1.0\"?>\r\n<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body><u:{action} xmlns:u=\"{service_type}\">{args}</u:{action}></s:Body></s:Envelope>"
    )
}

async fn upnp_soap(
    control_url: &str,
    service_type: &str,
    action: &str,
    args: &str,
) -> Option<String> {
    let url = parse_http_url(control_url)?;
    let body = soap_envelope(service_type, action, args);
    let request = format!(
        "POST {} HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: text/xml; charset=\"utf-8\"\r\nSOAPAction: \"{}#{}\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        url.path,
        url.host,
        url.port,
        service_type,
        action,
        body.len(),
        body
    );
    http_exchange(&url, &request).await
}

async fn upnp_delete_mapping(
    control_url: &str,
    service_type: &str,
    external_port: u16,
) -> Option<()> {
    let args = format!(
        "<NewRemoteHost></NewRemoteHost><NewExternalPort>{external_port}</NewExternalPort><NewProtocol>TCP</NewProtocol>"
    );
    upnp_soap(control_url, service_type, "DeletePortMapping", &args)
        .await
        .map(|_| ())
}

async fn upnp_map(local: Ipv4Addr, internal_port: u16) -> Option<DirectAccess> {
    let location = upnp_discover_location().await?;
    upnp_map_at(&location, local, internal_port).await
}

async fn upnp_map_at(location: &str, local: Ipv4Addr, internal_port: u16) -> Option<DirectAccess> {
    let url = parse_http_url(location)?;
    let get = format!(
        "GET {} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\n\r\n",
        url.path, url.host, url.port
    );
    let description = http_exchange(&url, &get).await?;
    let service = select_igd_service(&description, location)?;
    let external_text = upnp_soap(
        &service.control_url,
        &service.service_type,
        "GetExternalIPAddress",
        "",
    )
    .await?;
    let external_ip = parse_external_ip_soap(&external_text)?;
    if classify_ipv4(external_ip) != IpClass::Public {
        return None;
    }
    for step in 0..5u16 {
        let Some(external_port) = internal_port.checked_add(step) else {
            break;
        };
        let args = format!(
            "<NewRemoteHost></NewRemoteHost><NewExternalPort>{external_port}</NewExternalPort><NewProtocol>TCP</NewProtocol><NewInternalPort>{internal_port}</NewInternalPort><NewInternalClient>{local}</NewInternalClient><NewEnabled>1</NewEnabled><NewPortMappingDescription>PairProof session</NewPortMappingDescription><NewLeaseDuration>{}</NewLeaseDuration>",
            0
        );
        let Some(response) = upnp_soap(
            &service.control_url,
            &service.service_type,
            "AddPortMapping",
            &args,
        )
        .await
        else {
            continue;
        };
        if response.contains("errorCode") || response.contains(":Fault") {
            continue;
        }
        let endpoint = Endpoint::public(&external_ip.to_string(), external_port).ok()?;
        return Some(DirectAccess {
            endpoint,
            mapping: Some(PortMapping::Upnp {
                control_url: service.control_url.clone(),
                service_type: service.service_type.clone(),
                external_port,
            }),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, UdpSocket};

    #[tokio::test]
    async fn natpmp_fake_gateway_maps_port() {
        let server = UdpSocket::bind((Ipv4Addr::LOCALHOST, NATPMP_PORT))
            .await
            .expect("bind fake NAT-PMP gateway");
        let task = tokio::spawn(async move {
            let mut buf = [0u8; 64];
            // External-address request -> reply with a public WAN address.
            let (n, peer) = server.recv_from(&mut buf).await.expect("recv ext");
            assert_eq!(&buf[..n], &[0, 0]);
            let mut resp = vec![0u8, 128, 0, 0, 0, 0, 0, 1];
            resp.extend_from_slice(&Ipv4Addr::new(8, 8, 4, 4).octets());
            server.send_to(&resp, peer).await.expect("send ext");
            // Mapping request -> assign the requested external port.
            let (_n, peer) = server.recv_from(&mut buf).await.expect("recv map");
            assert_eq!(buf[1], 1);
            let mut resp = vec![0u8, 129, 0, 0, 0, 0, 0, 1];
            resp.extend_from_slice(&buf[4..6]);
            resp.extend_from_slice(&buf[6..8]);
            resp.extend_from_slice(&7200u32.to_be_bytes());
            server.send_to(&resp, peer).await.expect("send map");
        });
        let outcome = natpmp_map_at(Ipv4Addr::LOCALHOST, 2222).await;
        task.await.expect("fake gateway task");
        match outcome {
            NatpmpOutcome::Mapped(access) => {
                assert_eq!(access.endpoint().host(), "8.8.4.4");
                assert_eq!(access.endpoint().port(), 2222);
                assert!(access.mapping.is_some());
            }
            _ => panic!("expected a NAT-PMP mapping"),
        }
    }

    #[tokio::test]
    async fn upnp_fake_igd_maps_port() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind fake IGD");
        let addr = listener.local_addr().expect("local addr");
        let task = tokio::spawn(async move {
            let mut saw_mapping = false;
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().await.expect("accept");
                let mut buf = vec![0u8; 8192];
                let n = stream.read(&mut buf).await.expect("read request");
                let request = String::from_utf8_lossy(&buf[..n]).into_owned();
                let body = if request.starts_with("GET ") {
                    "<root><device><deviceList><device><serviceList><service><serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</serviceType><controlURL>/ctl</controlURL></service></serviceList></device></deviceList></device></root>".to_string()
                } else if request.contains("GetExternalIPAddress") {
                    "<s:Envelope><s:Body><u:GetExternalIPAddressResponse><NewExternalIPAddress>8.8.4.4</NewExternalIPAddress></u:GetExternalIPAddressResponse></s:Body></s:Envelope>".to_string()
                } else {
                    assert!(request.contains("AddPortMapping"), "{request}");
                    assert!(
                        request.contains("<NewInternalClient>192.168.1.5</NewInternalClient>"),
                        "{request}"
                    );
                    saw_mapping = true;
                    "<s:Envelope><s:Body><u:AddPortMappingResponse/></s:Body></s:Envelope>"
                        .to_string()
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(response.as_bytes()).await.expect("write");
            }
            saw_mapping
        });
        let location = format!("http://{}/desc.xml", addr);
        let access = upnp_map_at(&location, Ipv4Addr::new(192, 168, 1, 5), 2222)
            .await
            .expect("UPnP mapping");
        assert_eq!(access.endpoint().host(), "8.8.4.4");
        assert_eq!(access.endpoint().port(), 2222);
        assert!(task.await.expect("fake IGD task"));
    }
}
