use pairproof::distribution::{parse_version, DistributionError};

#[test]
fn version_comparison_handles_equal_versions() {
    let a = parse_version("1.2.3").expect("valid version");
    let b = parse_version("v1.2.3").expect("valid version with v prefix");
    assert_eq!(a, b);
    assert_eq!(a.to_string(), "1.2.3");
    // Build metadata is ignored for precedence and equality.
    let with_build = parse_version("1.2.3+build.42").expect("valid build metadata");
    assert_eq!(a, with_build);
    assert!(!a.is_prerelease());
}

#[test]
fn version_comparison_handles_newer_versions() {
    let cases = [
        ("1.2.4", "1.2.3"),
        ("1.3.0", "1.2.9"),
        ("2.0.0", "1.9.9"),
        ("1.10.0", "1.9.9"),
        ("0.1.1", "0.1.0"),
    ];
    for (newer, older) in cases {
        let newer = parse_version(newer).expect("valid newer version");
        let older = parse_version(older).expect("valid older version");
        assert!(newer > older, "{newer} should be > {older}");
        assert!(older < newer);
    }
}

#[test]
fn version_comparison_handles_prereleases() {
    let chain = [
        "1.0.0-alpha",
        "1.0.0-alpha.1",
        "1.0.0-alpha.beta",
        "1.0.0-beta",
        "1.0.0-beta.2",
        "1.0.0-beta.11",
        "1.0.0-rc.1",
        "1.0.0",
    ];
    for pair in chain.windows(2) {
        let lower = parse_version(pair[0]).expect("valid prerelease");
        let higher = parse_version(pair[1]).expect("valid prerelease");
        assert!(lower < higher, "{lower} should be < {higher}");
    }
    // Numeric identifiers compare below alphanumeric ones.
    let numeric = parse_version("1.0.0-1").expect("numeric prerelease");
    let alphanumeric = parse_version("1.0.0-alpha").expect("alphanumeric prerelease");
    assert!(numeric < alphanumeric);
    assert!(parse_version("1.0.0-rc.1")
        .expect("prerelease")
        .is_prerelease());
}

#[test]
fn version_comparison_rejects_invalid_versions() {
    let invalid = [
        "",
        "v",
        "1",
        "1.2",
        "1.2.3.4",
        "a.b.c",
        "1.02.3",
        "01.2.3",
        "1.2.3-",
        "1.2.3-alpha..1",
        "1.2.3-alpha_1",
        "1.2.3-01",
        "1.2.3+",
        "-1.2.3",
        "1.-2.3",
    ];
    for input in invalid {
        let result = parse_version(input);
        assert!(
            matches!(result, Err(DistributionError::InvalidVersion(_))),
            "expected InvalidVersion for {input:?}, got {result:?}"
        );
    }
}
