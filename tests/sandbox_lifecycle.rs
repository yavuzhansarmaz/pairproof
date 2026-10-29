use pairproof::connection::{Endpoint, EphemeralUsername, Secret};
use pairproof::sandbox::command::image_tag;
use pairproof::sandbox::error::SandboxError;
use pairproof::sandbox::image::{
    effective_image_tag, embedded_build_context, ensure_image, image_present,
};
use pairproof::sandbox::lifecycle::{
    destroy, destroy_by_session_id, start_container, SandboxHandle,
};

fn context_text(name: &str) -> &'static str {
    embedded_build_context()
        .into_iter()
        .find(|(n, _)| *n == name)
        .map(|(_, text)| text)
        .expect("embedded file present")
}

#[test]
fn ids_derives_names_and_port() {
    let endpoint = Endpoint::new("127.0.0.1", 2222).expect("endpoint");
    let handle = SandboxHandle::ids("s1", &endpoint).expect("handle");
    assert_eq!(handle.session_id(), "s1");
    assert_eq!(handle.container_name(), "pairproof-s1");
    assert_eq!(handle.network_name(), "pairproof-net-s1");
    assert_eq!(handle.endpoint_port(), 2222);
}

#[test]
fn ids_rejects_invalid_session() {
    let endpoint = Endpoint::new("127.0.0.1", 2222).expect("endpoint");
    for bad in ["", "-x", "a/b", "a b", "x;rm", "net-foo", "--privileged"] {
        let err = SandboxHandle::ids(bad, &endpoint).expect_err("invalid session must fail");
        assert!(
            matches!(err, SandboxError::InvalidSessionId(_)),
            "bad {bad:?} gave {err:?}"
        );
    }
}

#[test]
fn embedded_context_has_three_files_with_hardening_markers() {
    let files = embedded_build_context();
    assert_eq!(files.len(), 3);
    assert_eq!(files[0].0, "Dockerfile");
    assert_eq!(files[1].0, "entrypoint.sh");
    assert_eq!(files[2].0, "sshd_config");
    let dockerfile = context_text("Dockerfile");
    assert!(dockerfile.contains("rm -f /etc/ssh/ssh_host_"));
    assert!(!dockerfile.contains("PRIVATE KEY"));
    assert!(dockerfile.contains("mariadb-server"));
    assert!(dockerfile.contains("iptables"));
    assert!(dockerfile.contains("openssh-server"));
    let sshd = context_text("sshd_config");
    for marker in [
        "PermitRootLogin no",
        "PasswordAuthentication yes",
        "PubkeyAuthentication no",
        "AllowTcpForwarding no",
        "X11Forwarding no",
        "AllowAgentForwarding no",
        "PermitTunnel no",
    ] {
        assert!(sshd.contains(marker), "sshd_config missing {marker}");
    }
    let entrypoint = context_text("entrypoint.sh");
    assert!(entrypoint.contains("ssh-keygen -A"));
    assert!(entrypoint.contains("skip-networking"));
    assert!(entrypoint.contains("exec /usr/sbin/sshd"));
    for (_, text) in files {
        assert!(!text.contains("MODEL_API_KEY"));
        assert!(!text.contains("/home/"));
    }
}

#[test]
fn dockerfile_has_no_user_or_secret_baked() {
    let dockerfile = context_text("Dockerfile");
    assert!(!dockerfile.contains("useradd"));
    assert!(!dockerfile.contains("chpasswd"));
    assert!(!dockerfile.contains("pairproof:v1"));
    let entrypoint = context_text("entrypoint.sh");
    assert!(!entrypoint.contains("useradd"));
    assert!(!entrypoint.contains("chpasswd"));
}

#[test]
fn effective_tag_extends_base_tag() {
    let base = image_tag();
    let effective = effective_image_tag();
    let prefix = format!("{base}-");
    assert!(effective.starts_with(&prefix), "effective tag: {effective}");
    let suffix = &effective[prefix.len()..];
    assert_eq!(suffix.len(), 16, "suffix: {suffix}");
    assert!(
        suffix
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "suffix must be lowercase hex: {suffix}"
    );
}

#[test]
fn effective_tag_is_stable() {
    assert_eq!(effective_image_tag(), effective_image_tag());
}

#[tokio::test]
async fn destroy_by_session_id_rejects_invalid_id_without_docker() {
    let err = destroy_by_session_id("a/b;rm")
        .await
        .expect_err("invalid id must fail before docker");
    assert!(matches!(err, SandboxError::InvalidSessionId(_)));
    let err = destroy_by_session_id("")
        .await
        .expect_err("empty id must fail before docker");
    assert!(matches!(err, SandboxError::InvalidSessionId(_)));
}

#[test]
fn availability_error_types_are_distinct() {
    let not_found = SandboxError::DockerNotFound.to_string();
    let daemon = SandboxError::DaemonUnavailable {
        stderr: "cannot connect".to_string(),
    }
    .to_string();
    assert_ne!(not_found, daemon);
    assert!(not_found.contains("docker"));
    assert!(daemon.contains("daemon"));
    assert!(daemon.contains("cannot connect"));
    assert_ne!(
        SandboxError::DockerNotFound,
        SandboxError::DaemonUnavailable {
            stderr: String::new()
        }
    );
}

/// Extracts the password-hash field for `user` from `cat /etc/shadow` output.
fn shadow_hash(shadow: &str, user: &str) -> String {
    let prefix = format!("{user}:");
    let line = shadow
        .lines()
        .find(|l| l.starts_with(&prefix))
        .expect("ephemeral user line in /etc/shadow");
    line.split(':')
        .nth(1)
        .expect("shadow hash field")
        .to_string()
}

#[tokio::test]
async fn image_present_invalid_tag_fails_before_docker() {
    let err = image_present("--format=x")
        .await
        .expect_err("flag-like tag must fail validation");
    assert!(
        matches!(err, SandboxError::InvalidCommand(_)),
        "got {err:?}"
    );
}

#[tokio::test]
async fn ensure_image_invalid_tag_fails_before_docker() {
    let err = ensure_image("--privileged")
        .await
        .expect_err("flag-like tag must fail validation");
    assert!(
        matches!(err, SandboxError::InvalidCommand(_)),
        "got {err:?}"
    );
}

#[tokio::test]
async fn start_container_invalid_tag_fails_before_docker() {
    let endpoint = Endpoint::new("127.0.0.1", 2222).expect("endpoint");
    let handle = SandboxHandle::ids("s1", &endpoint).expect("handle");
    let err = start_container(&handle, "--privileged")
        .await
        .expect_err("flag-like tag must fail validation");
    assert!(
        matches!(err, SandboxError::InvalidCommand(_)),
        "got {err:?}"
    );
}

#[tokio::test]
#[ignore = "live docker: builds the sandbox image and provisions a container"]
async fn live_create_provision_ssh_port_and_destroy() {
    use pairproof::sandbox::command::{exec_as, exec_root};
    use pairproof::sandbox::lifecycle::{create_and_provision, provision_user, rotate_secret};
    use pairproof::sandbox::preflight::port_is_free;
    use pairproof::sandbox::process::{run_bounded, DEFAULT_MAX_OUTPUT};
    use std::time::Duration;
    use tokio::io::AsyncReadExt;

    let session_id = format!("live{}", std::process::id());
    let port = (22100..22200u16)
        .find(|p| port_is_free(*p))
        .expect("a free port in 22100..22200");
    let endpoint = Endpoint::new("127.0.0.1", port).expect("endpoint");
    let user = EphemeralUsername::generate();
    let secret = Secret::generate();
    let created = create_and_provision(&session_id, &endpoint, &user, &secret).await;
    let (handle, fingerprint) = match created {
        Ok(pair) => pair,
        Err(err) => {
            let _ = destroy_by_session_id(&session_id).await;
            panic!("create_and_provision failed: {err}");
        }
    };
    let fingerprint_text = fingerprint.as_str().to_string();

    let cat_argv = ["cat".to_string(), "/etc/shadow".to_string()];
    let cat_spec = exec_root(handle.container_name(), &cat_argv, false).expect("shadow exec spec");
    let before_out = run_bounded(&cat_spec, None, Duration::from_secs(30), DEFAULT_MAX_OUTPUT)
        .await
        .expect("read /etc/shadow");
    assert!(before_out.success(), "cat /etc/shadow: {before_out:?}");
    let before = shadow_hash(&before_out.stdout_text(), user.as_str());
    let rotated = Secret::generate();
    rotate_secret(&handle, &user, &rotated)
        .await
        .expect("rotate secret");
    let after_out = run_bounded(&cat_spec, None, Duration::from_secs(30), DEFAULT_MAX_OUTPUT)
        .await
        .expect("re-read /etc/shadow");
    assert!(after_out.success(), "cat /etc/shadow: {after_out:?}");
    let after = shadow_hash(&after_out.stdout_text(), user.as_str());
    assert_ne!(before, after, "shadow hash must change after rotation");
    provision_user(&handle, &user, &rotated)
        .await
        .expect("re-provision is idempotent");
    let cached = ensure_image(&effective_image_tag())
        .await
        .expect("cached image");
    assert!(cached, "ensure_image must report the cached image");

    let mut banner = Vec::new();
    let mut banner_err = String::new();
    match tokio::time::timeout(
        Duration::from_secs(10),
        tokio::net::TcpStream::connect(("127.0.0.1", port)),
    )
    .await
    {
        Ok(Ok(mut stream)) => {
            let mut buf = [0u8; 64];
            match tokio::time::timeout(Duration::from_secs(10), stream.read(&mut buf)).await {
                Ok(Ok(n)) => banner.extend_from_slice(&buf[..n]),
                Ok(Err(e)) => banner_err = e.to_string(),
                Err(e) => banner_err = e.to_string(),
            }
        }
        Ok(Err(e)) => banner_err = e.to_string(),
        Err(e) => banner_err = e.to_string(),
    }

    let outbound_spec = exec_as(
        handle.container_name(),
        &user,
        &[
            "python3".to_string(),
            "-c".to_string(),
            "import socket;socket.setdefaulttimeout(3);socket.create_connection(('1.1.1.1',53))"
                .to_string(),
        ],
        false,
    )
    .expect("exec spec");
    let outbound = run_bounded(
        &outbound_spec,
        None,
        Duration::from_secs(30),
        DEFAULT_MAX_OUTPUT,
    )
    .await;
    let outbound_blocked = match &outbound {
        Ok(outcome) => !outcome.success(),
        Err(_) => false,
    };

    let first_destroy = destroy(&handle).await;
    let second_destroy = destroy(&handle).await;

    assert!(
        fingerprint_text.len() == 64
            && fingerprint_text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "fingerprint must be 64 lowercase hex chars: {fingerprint_text}"
    );
    assert!(
        banner.starts_with(b"SSH-"),
        "expected SSH banner on published port, got {banner:?} err={banner_err}"
    );
    assert!(
        outbound_blocked,
        "outbound connection from container must be blocked: {outbound:?}"
    );
    assert!(first_destroy.is_ok(), "first destroy: {first_destroy:?}");
    assert!(
        second_destroy.is_ok(),
        "second destroy must be idempotent: {second_destroy:?}"
    );
}
