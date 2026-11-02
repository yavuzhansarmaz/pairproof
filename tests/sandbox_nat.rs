use std::net::Ipv4Addr;

use pairproof::sandbox::nat::{
    build_natpmp_external_request, build_natpmp_mapping_request, build_stun_binding_request,
    extract_ssdp_location, parse_external_ip_soap, parse_natpmp_external_response,
    parse_natpmp_mapping_response, parse_route_table, parse_stun_binding_response,
    select_igd_service,
};

fn ip(a: u8, b: u8, c: u8, d: u8) -> Ipv4Addr {
    Ipv4Addr::new(a, b, c, d)
}

#[test]
fn stun_response_xor_mapped_roundtrip_shape() {
    let txn: [u8; 12] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
    let req = build_stun_binding_request(txn);
    assert_eq!(&req[0..4], &[0x00, 0x01, 0x00, 0x00]);
    // Build a XOR-MAPPED-ADDRESS response for 1.2.3.4:5000.
    let cookie: u32 = 0x2112_A442;
    let xip = u32::from(ip(1, 2, 3, 4)) ^ cookie;
    let xport: u16 = 5000 ^ 0x2112;
    let mut resp = Vec::new();
    resp.extend_from_slice(&0x0101u16.to_be_bytes());
    resp.extend_from_slice(&8u16.to_be_bytes());
    resp.extend_from_slice(&cookie.to_be_bytes());
    resp.extend_from_slice(&txn);
    resp.extend_from_slice(&0x0020u16.to_be_bytes());
    resp.extend_from_slice(&8u16.to_be_bytes());
    resp.push(0);
    resp.push(1);
    resp.extend_from_slice(&xport.to_be_bytes());
    resp.extend_from_slice(&xip.to_be_bytes());
    assert_eq!(
        parse_stun_binding_response(&resp, &txn),
        Some((ip(1, 2, 3, 4), 5000))
    );
    // Wrong transaction id is rejected.
    let mut bad = resp.clone();
    bad[8] ^= 0xFF;
    assert_eq!(parse_stun_binding_response(&bad, &txn), None);
}

#[test]
fn natpmp_parsers_validate_opcode_and_result() {
    assert_eq!(build_natpmp_external_request(), [0, 0]);
    let req = build_natpmp_mapping_request(2222, 2222, 7200);
    assert_eq!(req[1], 1);
    // External-address response: ver, op=128, result=0, epoch, addr.
    let mut ext = vec![0u8, 128, 0, 0, 0, 0, 0, 1];
    ext.extend_from_slice(&ip(8, 8, 4, 4).octets());
    assert_eq!(parse_natpmp_external_response(&ext), Some(ip(8, 8, 4, 4)));
    let mut failed = ext.clone();
    failed[3] = 2;
    assert_eq!(parse_natpmp_external_response(&failed), None);
    // Mapping response: ver, op=129, result=0, epoch, internal, mapped, lifetime.
    let mut map = vec![0u8, 129, 0, 0, 0, 0, 0, 1];
    map.extend_from_slice(&2222u16.to_be_bytes());
    map.extend_from_slice(&40000u16.to_be_bytes());
    map.extend_from_slice(&7200u32.to_be_bytes());
    assert_eq!(parse_natpmp_mapping_response(&map), Some(40000));
}

#[test]
fn route_table_picks_default_gateway() {
    let text = "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\n\
                eth0\t00000000\t0101A8C0\t0003\t0\t0\t0\t00000000\n\
                eth0\t0001A8C0\t00000000\t0001\t0\t0\t0\t00FFFFFF\n";
    assert_eq!(parse_route_table(text), Some(ip(192, 168, 1, 1)));
    assert_eq!(parse_route_table("Iface\tDestination\tGateway\n"), None);
}

#[test]
fn ssdp_location_and_igd_service_selection() {
    let resp = "HTTP/1.1 200 OK\r\nLOCATION: http://192.168.1.1:5000/rootDesc.xml\r\nST: urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\n\r\n";
    assert_eq!(
        extract_ssdp_location(resp).as_deref(),
        Some("http://192.168.1.1:5000/rootDesc.xml")
    );
    let xml = r#"<root><device><deviceList><device><deviceType>urn:schemas-upnp-org:device:WANConnectionDevice:1</deviceType><serviceList><service><serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</serviceType><controlURL>/ctl/IPConn</controlURL></service></serviceList></device></deviceList></device></root>"#;
    let svc = select_igd_service(xml, "http://192.168.1.1:5000/rootDesc.xml").expect("igd service");
    assert_eq!(svc.control_url, "http://192.168.1.1:5000/ctl/IPConn");
    assert!(svc.service_type.contains("WANIPConnection"));
    // Off-origin absolute control URLs are rejected.
    let evil = xml.replace("/ctl/IPConn", "http://10.0.0.9:9/evil");
    assert!(select_igd_service(&evil, "http://192.168.1.1:5000/rootDesc.xml").is_none());
}

#[test]
fn soap_external_ip_parse() {
    let body = "<s:Envelope><s:Body><u:GetExternalIPAddressResponse><NewExternalIPAddress>8.8.8.8</NewExternalIPAddress></u:GetExternalIPAddressResponse></s:Body></s:Envelope>";
    assert_eq!(parse_external_ip_soap(body), Some(ip(8, 8, 8, 8)));
    assert_eq!(parse_external_ip_soap("<x/>"), None);
}
