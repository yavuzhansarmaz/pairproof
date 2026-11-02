use pairproof::connection::{Endpoint, EphemeralUsername, Fingerprint, Secret};
use pairproof::orchestration::ssh_auth::{
    helper_script, run_ssh_command, ssh_invocation, verify_and_key_line, AskpassSession,
    SshAuthError,
};
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::time::{Duration, Instant};

const CANARY: &str = "0123456789abcdeffedcba9876543210";
const KEY_LINE: &str =
    "127.0.0.1 ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOr8qXRxd07w1+T+R1EJccWCxO7Z82oSjs0+Knb0KoAA";

fn mode(path: &std::path::Path) -> u32 {
    std::fs::metadata(path)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777
}
fn user() -> EphemeralUsername {
    EphemeralUsername::parse("pp-a1b2c3d4").expect("user")
}
fn count_ssh_sessions() -> usize {
    std::fs::read_dir(std::env::temp_dir())
        .expect("temp dir")
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("pairproof-askpass-ssh-")
        })
        .count()
}

#[test]
fn askpass_dir_has_private_permissions() {
    use std::os::unix::fs::FileTypeExt;
    let session = AskpassSession::create("perms").expect("session");
    assert_eq!(mode(session.dir_path()), 0o700);
    assert_eq!(mode(session.helper_path()), 0o700);
    assert_eq!(mode(session.fifo_path()), 0o600);
    assert!(std::fs::metadata(session.fifo_path())
        .expect("fifo metadata")
        .file_type()
        .is_fifo());
}

#[test]
fn askpass_helper_answers_only_password_prompts() {
    let session = AskpassSession::create("prompts").expect("session");
    let denied = Command::new(session.helper_path())
        .arg("Are you sure you want to continue connecting (yes/no/[fingerprint])?")
        .output()
        .expect("helper runs");
    assert!(!denied.status.success());
    assert!(denied.stdout.is_empty());
    let secret = Secret::from_hex(CANARY).expect("canary");
    let feeder = session.feed_password(&secret);
    let allowed = Command::new(session.helper_path())
        .arg("pp-a1b2c3d4@127.0.0.1's password:")
        .output()
        .expect("helper runs");
    feeder.stop();
    assert!(allowed.status.success());
    assert_eq!(String::from_utf8_lossy(&allowed.stdout).trim(), CANARY);
}

#[test]
fn askpass_helper_contains_no_secret() {
    let session = AskpassSession::create("nosecret").expect("session");
    let text = std::fs::read_to_string(session.helper_path()).expect("helper text");
    assert_eq!(text, helper_script(session.fifo_path()));
    assert!(text.contains("*assword*"));
    assert!(text.contains("exit 1"));
    assert!(!text.contains(CANARY));
}

#[test]
fn ssh_auth_secret_never_in_argv_env_or_debug() {
    let session = AskpassSession::create("argv").expect("session");
    let endpoint = Endpoint::new("127.0.0.1", 2222).expect("endpoint");
    let secret = Secret::from_hex(CANARY).expect("canary");
    let invocation = ssh_invocation(
        &session,
        &endpoint,
        &user(),
        &["echo", "AUTH_OK"],
        Duration::from_secs(5),
    );
    let argv = invocation.args.join(" ");
    let env = invocation
        .env
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(" ");
    assert!(!argv.contains(CANARY));
    assert!(!env.contains(CANARY));
    assert!(argv.contains("StrictHostKeyChecking=yes"));
    assert!(argv.contains("PubkeyAuthentication=no"));
    assert!(!format!("{session:?}").contains(CANARY));
    assert!(!format!("{invocation:?}").contains(CANARY));
    let feeder = session.feed_password(&secret);
    assert!(!format!("{feeder:?}").contains(CANARY));
    feeder.stop();
    let err = SshAuthError::Io(CANARY.to_string());
    assert!(!format!("{err:?}").contains(CANARY) && !format!("{err}").contains(CANARY));
    let err = SshAuthError::HostKeyUnavailable(CANARY.to_string());
    assert!(!format!("{err:?}").contains(CANARY) && !format!("{err}").contains(CANARY));
}

#[test]
fn askpass_session_cleanup_on_drop() {
    let dir = {
        let session = AskpassSession::create("cleanup").expect("session");
        let dir = session.dir_path().to_path_buf();
        assert!(dir.exists());
        dir
    };
    assert!(!dir.exists());
}

#[test]
fn askpass_session_cleanup_after_failure() {
    let session = AskpassSession::create("failcleanup").expect("session");
    let dir = session.dir_path().to_path_buf();
    assert!(session.write_known_hosts("bad\nline").is_err());
    let denied = Command::new(session.helper_path())
        .arg("confirm?")
        .output()
        .expect("helper");
    assert!(!denied.status.success());
    drop(session);
    assert!(!dir.exists());
    let endpoint = Endpoint::new("127.0.0.1", 1).expect("endpoint");
    let secret = Secret::from_hex(CANARY).expect("canary");
    let result = run_ssh_command(
        &endpoint,
        &user(),
        &secret,
        KEY_LINE,
        &["echo", "AUTH_OK"],
        Duration::from_secs(5),
    );
    assert!(result.is_err());
    // No ssh-tagged session may persist; a parallel timeout test drains to 0 too.
    let deadline = Instant::now() + Duration::from_secs(8);
    while count_ssh_sessions() > 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(count_ssh_sessions(), 0);
}

#[test]
fn known_hosts_written_only_from_verified_line() {
    let session = AskpassSession::create("knownhosts").expect("session");
    session.write_known_hosts(KEY_LINE).expect("write");
    assert_eq!(
        std::fs::read_to_string(session.known_hosts_path()).expect("read"),
        format!("{KEY_LINE}\n")
    );
    assert_eq!(mode(session.known_hosts_path()), 0o600);
    assert!(session.write_known_hosts("").is_err());
    assert!(session.write_known_hosts("a\nb").is_err());
    assert_eq!(
        std::fs::read_to_string(session.known_hosts_path()).expect("read"),
        format!("{KEY_LINE}\n")
    );
}

#[test]
fn ssh_command_times_out_bounded() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("listener");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming() {
            match stream {
                Ok(s) => held.push(s),
                Err(_) => break,
            }
            if held.len() >= 2 {
                break;
            }
        }
        std::thread::sleep(Duration::from_secs(3));
    });
    let endpoint = Endpoint::new("127.0.0.1", port).expect("endpoint");
    let secret = Secret::from_hex(CANARY).expect("canary");
    let started = Instant::now();
    let result = run_ssh_command(
        &endpoint,
        &user(),
        &secret,
        KEY_LINE,
        &["echo", "AUTH_OK"],
        Duration::from_millis(800),
    );
    assert_eq!(result, Err(SshAuthError::Timeout));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
#[ignore = "live docker+ssh: provisions a real sandbox"]
async fn live_fifo_ssh_password_auth_succeeds() {
    use pairproof::sandbox::lifecycle::{create_and_provision, destroy, destroy_by_session_id};
    use pairproof::sandbox::preflight::port_is_free;
    let session_id = format!("sshlive{}x{}", std::process::id(), 1);
    let port = (22100..22300u16)
        .find(|p| port_is_free(*p))
        .expect("free port");
    let endpoint = Endpoint::new("127.0.0.1", port).expect("endpoint");
    let username = EphemeralUsername::generate();
    let secret = Secret::generate();
    let (handle, fingerprint) =
        match create_and_provision(&session_id, &endpoint, &username, &secret).await {
            Ok(pair) => pair,
            Err(err) => {
                let _ = destroy_by_session_id(&session_id).await;
                panic!("create_and_provision failed: {err}");
            }
        };
    let line = verify_and_key_line(&endpoint, &fingerprint);
    let auth = line
        .as_ref()
        .map_err(|e| format!("{e:?}"))
        .and_then(|line| {
            run_ssh_command(
                &endpoint,
                &username,
                &secret,
                line,
                &["echo", "AUTH_OK"],
                Duration::from_secs(20),
            )
            .map_err(|e| format!("{e:?}"))
        });
    let wrong_fp = Fingerprint::parse(&"b".repeat(64)).expect("fp");
    let mismatch = if fingerprint.as_str() == wrong_fp.as_str() {
        Ok(())
    } else {
        verify_and_key_line(&endpoint, &wrong_fp).map(|_| ())
    };
    let destroyed = destroy(&handle).await;
    assert!(destroyed.is_ok(), "destroy: {destroyed:?}");
    let line = line.expect("verified key line");
    assert!(line.contains("ssh-ed25519"));
    let out = auth.expect("ssh auth");
    assert!(out.contains("AUTH_OK"), "ssh output: {out}");
    assert_eq!(mismatch, Err(SshAuthError::HostKeyMismatch));
}

#[tokio::test]
#[ignore = "live docker+ssh: provisions a real sandbox"]
async fn live_fifo_ssh_wrong_password_rejected() {
    use pairproof::sandbox::lifecycle::{create_and_provision, destroy, destroy_by_session_id};
    use pairproof::sandbox::preflight::port_is_free;
    let session_id = format!("sshlive{}x{}", std::process::id(), 2);
    let port = (22300..22500u16)
        .find(|p| port_is_free(*p))
        .expect("free port");
    let endpoint = Endpoint::new("127.0.0.1", port).expect("endpoint");
    let username = EphemeralUsername::generate();
    let secret = Secret::generate();
    let (handle, fingerprint) =
        match create_and_provision(&session_id, &endpoint, &username, &secret).await {
            Ok(pair) => pair,
            Err(err) => {
                let _ = destroy_by_session_id(&session_id).await;
                panic!("create_and_provision failed: {err}");
            }
        };
    let line = verify_and_key_line(&endpoint, &fingerprint);
    let mut wrong = Secret::generate();
    while wrong == secret {
        wrong = Secret::generate();
    }
    let auth = line
        .as_ref()
        .map_err(|e| format!("{e:?}"))
        .and_then(|line| {
            run_ssh_command(
                &endpoint,
                &username,
                &wrong,
                line,
                &["echo", "AUTH_OK"],
                Duration::from_secs(20),
            )
            .map_err(|e| format!("{e:?}"))
        });
    let destroyed = destroy(&handle).await;
    assert!(destroyed.is_ok(), "destroy: {destroyed:?}");
    assert!(line.is_ok(), "key line: {line:?}");
    assert_eq!(auth, Err("AuthFailed".to_string()));
}
