use pairproof::connection::{
    ConnectionError, ConnectionString, Endpoint, EphemeralUsername, Fingerprint, Secret,
};

const SECRET_HEX: &str = "0123456789abcdef0123456789abcdef";
const FINGERPRINT_HEX: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const CANONICAL: &str = "pairproof:v1:203.0.113.10:22022:pp-a1b2c3d4:\
0123456789abcdef0123456789abcdef:1730000060000:\
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn sample() -> ConnectionString {
    ConnectionString::new(
        Endpoint::new("203.0.113.10", 22022).expect("endpoint"),
        EphemeralUsername::parse("pp-a1b2c3d4").expect("username"),
        Secret::from_hex(SECRET_HEX).expect("secret"),
        1_730_000_060_000,
        Fingerprint::parse(FINGERPRINT_HEX).expect("fingerprint"),
    )
}

fn with_field(index: usize, value: &str) -> String {
    let mut fields: Vec<&str> = CANONICAL.split(':').collect();
    fields[index] = value;
    fields.join(":")
}

#[test]
fn round_trip_preserves_all_fields() {
    let original = sample();
    let parsed = ConnectionString::parse(&original.encode()).expect("parse");
    assert_eq!(parsed, original);
    assert_eq!(parsed.endpoint().host(), "203.0.113.10");
    assert_eq!(parsed.endpoint().port(), 22022);
    assert_eq!(parsed.username().as_str(), "pp-a1b2c3d4");
    assert_eq!(parsed.secret().expose_hex(), SECRET_HEX);
    assert_eq!(parsed.expires_at_ms(), 1_730_000_060_000);
    assert_eq!(parsed.fingerprint().as_str(), FINGERPRINT_HEX);
}

#[test]
fn round_trip_over_generated_values() {
    for i in 0..50 {
        let original = ConnectionString::new(
            Endpoint::new("example.com", 22022).expect("endpoint"),
            EphemeralUsername::generate(),
            Secret::generate(),
            1_730_000_060_000 + i,
            Fingerprint::parse(&"ab".repeat(32)).expect("fingerprint"),
        );
        let parsed = ConnectionString::parse(&original.encode()).expect("parse");
        assert_eq!(parsed, original);
    }
}

#[test]
fn encode_matches_canonical_example() {
    let bytes = [
        0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd,
        0xef,
    ];
    let cs = ConnectionString::new(
        Endpoint::new("203.0.113.10", 22022).expect("endpoint"),
        EphemeralUsername::parse("pp-a1b2c3d4").expect("username"),
        Secret::from_bytes(bytes),
        1_730_000_060_000,
        Fingerprint::parse(FINGERPRINT_HEX).expect("fingerprint"),
    );
    assert_eq!(cs.encode(), CANONICAL);
}

#[test]
fn parse_rejects_extra_field() {
    assert!(ConnectionString::parse(&format!("{CANONICAL}:extra")).is_err());
}

#[test]
fn parse_rejects_missing_field() {
    let fields: Vec<&str> = CANONICAL.split(':').collect();
    assert!(ConnectionString::parse(&fields[..7].join(":")).is_err());
    assert!(ConnectionString::parse("pairproof:v1:203.0.113.10").is_err());
}

#[test]
fn parse_rejects_wrong_version() {
    let err = ConnectionString::parse(&with_field(1, "v2")).expect_err("must reject");
    assert_eq!(err, ConnectionError::UnsupportedVersion("v2".to_string()));
}

#[test]
fn parse_rejects_wrong_prefix() {
    assert!(ConnectionString::parse(&with_field(0, "other")).is_err());
    assert!(ConnectionString::parse(&with_field(0, "PairProof")).is_err());
}

#[test]
fn parse_rejects_whitespace_anywhere() {
    for raw in [
        format!(" {CANONICAL}"),
        format!("{CANONICAL} "),
        format!("{CANONICAL}\n"),
        CANONICAL.replace("22022", "22 022"),
        CANONICAL.replace("pp-a1b2c3d4", "pp-a1b2\tc3d4"),
        CANONICAL.replace("203.0.113.10", "203.0.113.\u{7}10"),
    ] {
        assert!(ConnectionString::parse(&raw).is_err(), "accepted {raw:?}");
    }
}

#[test]
fn parse_rejects_empty_and_short_secret() {
    assert!(ConnectionString::parse("").is_err());
    assert_eq!(
        ConnectionString::parse(&with_field(5, "")).expect_err("empty secret"),
        ConnectionError::InvalidSecret
    );
    assert_eq!(
        ConnectionString::parse(&with_field(5, "0123456789abcdef")).expect_err("short secret"),
        ConnectionError::InvalidSecret
    );
    assert_eq!(
        ConnectionString::parse(&with_field(5, &SECRET_HEX.to_uppercase()))
            .expect_err("uppercase secret"),
        ConnectionError::InvalidSecret
    );
}

#[test]
fn endpoint_rejects_port_zero() {
    assert!(matches!(
        Endpoint::new("203.0.113.10", 0),
        Err(ConnectionError::InvalidEndpoint(_))
    ));
    assert!(ConnectionString::parse(&with_field(3, "0")).is_err());
}

#[test]
fn endpoint_rejects_bad_hosts() {
    for host in [
        "",
        "::1",
        "999.1.1.1",
        "bad_host",
        "-bad",
        "bad-",
        "example.com.",
        "0.0.0.0",
        "1.2.3",
        "a..b",
        ".example.com",
    ] {
        assert!(
            matches!(
                Endpoint::new(host, 22022),
                Err(ConnectionError::InvalidEndpoint(_))
            ),
            "accepted host {host:?}"
        );
    }
}

#[test]
fn endpoint_accepts_ipv4_and_hostname() {
    let ip = Endpoint::new("203.0.113.10", 22022).expect("ipv4");
    assert_eq!(ip.host(), "203.0.113.10");
    assert_eq!(ip.port(), 22022);
    let name = Endpoint::new("EXAMPLE.com", 443).expect("hostname");
    assert_eq!(name.host(), "example.com");
    let sub = Endpoint::new("a-b1.Host-2.Example.ORG", 1).expect("labels");
    assert_eq!(sub.host(), "a-b1.host-2.example.org");
    assert_eq!(sub.port(), 1);
    assert!(Endpoint::new("localhost", 65535).is_ok());
}

#[test]
fn username_validation() {
    assert_eq!(
        EphemeralUsername::parse("pp-a1b2c3d4")
            .expect("valid")
            .as_str(),
        "pp-a1b2c3d4"
    );
    for raw in [
        "a1b2c3d4",
        "alice",
        "expert-user1",
        "pp-abc",
        "pp-A1B2C3D4",
        "pp-a1b2c3d4e",
        "pp-a1b2c3!",
        "PP-a1b2c3d4",
        "",
    ] {
        assert!(
            matches!(
                EphemeralUsername::parse(raw),
                Err(ConnectionError::InvalidUsername(_))
            ),
            "accepted username {raw:?}"
        );
    }
}

#[test]
fn generated_usernames_match_pattern_and_differ() {
    let a = EphemeralUsername::generate();
    let b = EphemeralUsername::generate();
    assert!(EphemeralUsername::parse(a.as_str()).is_ok());
    assert!(EphemeralUsername::parse(b.as_str()).is_ok());
    assert_ne!(a, b);
}

#[test]
fn fingerprint_validation() {
    assert!(Fingerprint::parse(&"ab".repeat(32)).is_ok());
    assert!(Fingerprint::parse(&"a".repeat(32)).is_ok());
    assert!(Fingerprint::parse(&"a".repeat(128)).is_ok());
    for raw in [
        "zz".repeat(32),
        "a".repeat(33),
        "a".repeat(16),
        "A".repeat(64),
        "a".repeat(130),
        String::new(),
    ] {
        assert!(
            matches!(
                Fingerprint::parse(&raw),
                Err(ConnectionError::InvalidFingerprint(_))
            ),
            "accepted fingerprint {raw:?}"
        );
    }
}

#[test]
fn secret_debug_is_redacted() {
    let secret = Secret::from_hex(SECRET_HEX).expect("secret");
    assert_eq!(format!("{secret:?}"), "Secret([REDACTED])");
    assert!(!format!("{secret:?}").contains(SECRET_HEX));
    assert!(!format!("{:?}", sample()).contains(SECRET_HEX));
}

#[test]
fn parse_rejects_noncanonical_numbers() {
    for raw in [
        with_field(3, "022022"),
        with_field(3, "70000"),
        with_field(3, "65536"),
        with_field(3, "+22022"),
        with_field(3, "-1"),
        with_field(3, ""),
        with_field(6, "01730000060000"),
        with_field(6, "99999999999999999999999999"),
        with_field(6, "-5"),
        with_field(6, ""),
    ] {
        assert!(ConnectionString::parse(&raw).is_err(), "accepted {raw}");
    }
    assert!(ConnectionString::parse(&with_field(6, "0")).is_ok());
}

#[test]
fn secret_equality() {
    let base = [7u8; 16];
    let mut diff_first = base;
    diff_first[0] ^= 1;
    let mut diff_last = base;
    diff_last[15] ^= 1;
    assert_eq!(Secret::from_bytes(base), Secret::from_bytes(base));
    assert_ne!(Secret::from_bytes(base), Secret::from_bytes(diff_first));
    assert_ne!(Secret::from_bytes(base), Secret::from_bytes(diff_last));
}

#[test]
fn display_and_fromstr_match_encode_and_parse() {
    let cs = sample();
    assert_eq!(format!("{cs}"), cs.encode());
    let via_from_str: ConnectionString = CANONICAL.parse().expect("FromStr");
    assert_eq!(
        via_from_str,
        ConnectionString::parse(CANONICAL).expect("parse")
    );
    assert_eq!(via_from_str, cs);
}

#[test]
fn parse_malformed_variant_for_shape_errors() {
    let fields: Vec<&str> = CANONICAL.split(':').collect();
    for raw in [
        format!(" {CANONICAL}"),
        format!("{CANONICAL}:extra"),
        fields[..7].join(":"),
        with_field(0, "other"),
        String::new(),
    ] {
        assert!(
            matches!(
                ConnectionString::parse(&raw),
                Err(ConnectionError::Malformed(_))
            ),
            "expected Malformed for {raw:?}, got {:?}",
            ConnectionString::parse(&raw)
        );
    }
}

#[test]
fn parse_invalid_expiry_variant_for_bad_expiry() {
    for raw in [
        with_field(6, "01730000060000"),
        with_field(6, "99999999999999999999999999"),
        with_field(6, ""),
    ] {
        assert!(
            matches!(
                ConnectionString::parse(&raw),
                Err(ConnectionError::InvalidExpiry(_))
            ),
            "expected InvalidExpiry for {raw:?}, got {:?}",
            ConnectionString::parse(&raw)
        );
    }
}
