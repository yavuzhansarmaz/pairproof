use pairproof::connection::{Endpoint, EphemeralUsername, Secret};
use pairproof::orchestration::mutagen_auth::{run_mutagen, MutagenAuthEnv};
use pairproof::orchestration::ssh_auth::{verify_and_key_line, AskpassSession, SshAuthError};
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

const CANARY: &str = "0123456789abcdeffedcba9876543210";
const KEY_LINE: &str =
    "127.0.0.1 ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOr8qXRxd07w1+T+R1EJccWCxO7Z82oSjs0+Knb0KoAA";
const WRONG_KEY_LINE: &str =
    "127.0.0.1 ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOr8qXRxd07w1+T+R1EJccWCxO7Z82oSjs0+Knb0KoAA";

fn mode(path: &std::path::Path) -> u32 {
    std::fs::metadata(path)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777
}
fn env_value<'a>(env: &'a MutagenAuthEnv, key: &str) -> &'a str {
    env.env_pairs()
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
        .expect("env key")
}

#[test]
fn mutagen_env_is_isolated_and_secret_free() {
    let session = AskpassSession::create("mutagenenv").expect("session");
    let env = MutagenAuthEnv::prepare(&session, KEY_LINE).expect("env");
    assert_eq!(env.data_dir(), session.dir_path().join("mutagen").as_path());
    assert!(env.data_dir().starts_with(session.dir_path()));
    assert_ne!(
        env_value(&env, "MUTAGEN_DATA_DIRECTORY"),
        format!("{}/.mutagen", std::env::var("HOME").unwrap_or_default())
    );
    assert!(env_value(&env, "PATH").starts_with(&env.bin_dir().to_string_lossy().to_string()));
    assert_eq!(env_value(&env, "SSH_ASKPASS_REQUIRE"), "force");
    assert_eq!(mode(env.data_dir()), 0o700);
    assert_eq!(mode(env.bin_dir()), 0o700);
    assert_eq!(mode(env.wrapper_path()), 0o700);
    for (_, value) in env.env_pairs() {
        assert!(!value.contains(CANARY));
    }
    assert!(!format!("{env:?}").contains(CANARY));
    let wrapper = std::fs::read_to_string(env.wrapper_path()).expect("wrapper");
    assert!(!wrapper.contains(CANARY));
}

#[test]
fn mutagen_wrapper_injects_pinned_host_key_options() {
    let session = AskpassSession::create("mutagenwrap").expect("session");
    let env = MutagenAuthEnv::prepare(&session, KEY_LINE).expect("env");
    let wrapper = std::fs::read_to_string(env.wrapper_path()).expect("wrapper");
    assert!(wrapper.contains("StrictHostKeyChecking=yes"));
    assert!(wrapper.contains(&format!(
        "UserKnownHostsFile={}",
        session.known_hosts_path().display()
    )));
    assert!(wrapper.contains("PreferredAuthentications=password"));
    assert!(wrapper.contains("PubkeyAuthentication=no"));
    assert!(wrapper.contains("SSH_ASKPASS_REQUIRE=force"));
    assert!(wrapper.contains("exec /"));
    assert!(wrapper.contains("/ssh"));
    assert!(!wrapper.contains("StrictHostKeyChecking=no"));
    assert!(!wrapper.contains("accept-new"));
    assert!(!wrapper.contains(CANARY));
    let scp = std::fs::read_to_string(env.scp_wrapper_path()).expect("scp wrapper");
    assert!(scp.contains("StrictHostKeyChecking=yes"));
    assert!(scp.contains("-S "));
    assert!(scp.contains(&env.wrapper_path().to_string_lossy().to_string()));
    assert!(!scp.contains(CANARY));
}

#[tokio::test]
#[ignore = "live docker+mutagen: provisions a real sandbox, requires ssh-keyscan on PATH"]
async fn live_mutagen_ssh_two_way_sync_with_password_auth() {
    use pairproof::sandbox::lifecycle::{create_and_provision, destroy, destroy_by_session_id};
    use pairproof::sandbox::preflight::port_is_free;
    let port = (22500..22700u16)
        .find(|p| port_is_free(*p))
        .expect("free port");
    let session_id = format!("mutlive{}x{}", std::process::id(), port);
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
    let alpha = std::env::temp_dir().join(format!("pairproof-alpha-{}-{port}", std::process::id()));
    let _ = std::fs::remove_dir_all(&alpha);
    std::fs::create_dir_all(&alpha).expect("alpha dir");
    std::fs::write(alpha.join("alpha.txt"), "alpha-content\n").expect("alpha file");
    let name = format!("pairproof-mutagen-{}-{port}", std::process::id());
    let beta = pairproof::sync::beta_url(&endpoint, &username);
    let mut evidence = Vec::new();
    let mut synced_alpha = false;
    let mut synced_beta = false;
    let mut resumed = false;
    let session = AskpassSession::create("mutagenlive").expect("askpass");
    let setup: Result<(), String> = (|| {
        let line =
            verify_and_key_line(&endpoint, &fingerprint).map_err(|e| format!("verify: {e:?}"))?;
        let env = MutagenAuthEnv::prepare(&session, &line).map_err(|e| format!("env: {e:?}"))?;
        let feeder = env.start_password_feeder(&secret);
        let daemon = run_mutagen(&env, &["daemon", "start"], Duration::from_secs(20))
            .map_err(|e| format!("daemon: {e:?}"))?;
        evidence.push(format!("daemon ok={}", daemon.success()));
        if !daemon.success() {
            return Err(format!("daemon failed: {}", daemon.stderr));
        }
        let created = run_mutagen(
            &env,
            &[
                "sync",
                "create",
                "--name",
                &name,
                "--mode",
                "two-way-safe",
                "--symlink-mode",
                "ignore",
                &alpha.to_string_lossy(),
                &beta,
            ],
            Duration::from_secs(60),
        )
        .map_err(|e| format!("create: {e:?}"))?;
        evidence.push(format!(
            "create ok={} err={}",
            created.success(),
            created.stderr.trim()
        ));
        if !created.success() {
            feeder.stop();
            let _ = run_mutagen(&env, &["daemon", "stop"], Duration::from_secs(10));
            return Err("sync create failed".into());
        }
        let _ = run_mutagen(&env, &["sync", "flush", &name], Duration::from_secs(30));
        // Poll alpha->beta with docker exec while env/feeder stay alive here.
        let container = handle.container_name().to_string();
        for _ in 0..30 {
            let out = std::process::Command::new("docker")
                .args(["exec", &container, "cat", "/workspace/alpha.txt"])
                .output()
                .expect("docker exec");
            if String::from_utf8_lossy(&out.stdout).contains("alpha-content") {
                synced_alpha = true;
                break;
            }
            let _ = run_mutagen(&env, &["sync", "flush", &name], Duration::from_secs(20));
            std::thread::sleep(Duration::from_secs(1));
        }

        // beta -> alpha: write in container via tee stdin, flush, poll local.
        let mut tee = std::process::Command::new("docker")
            .args(["exec", "-i", &container, "tee", "/workspace/beta.txt"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("tee");
        use std::io::Write as _;
        if let Some(mut stdin) = tee.stdin.take() {
            let _ = stdin.write_all(b"beta-content\n");
        }
        let _ = tee.wait();
        let _ = std::process::Command::new("docker")
            .args([
                "exec",
                &container,
                "chown",
                &format!("{}:{}", username.as_str(), username.as_str()),
                "/workspace/beta.txt",
            ])
            .output();
        for _ in 0..30 {
            let _ = run_mutagen(&env, &["sync", "flush", &name], Duration::from_secs(20));
            if alpha.join("beta.txt").exists()
                && std::fs::read_to_string(alpha.join("beta.txt"))
                    .map(|t| t.contains("beta-content"))
                    .unwrap_or(false)
            {
                synced_beta = true;
                break;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        let _ = run_mutagen(&env, &["sync", "pause", &name], Duration::from_secs(20));
        let resume = run_mutagen(&env, &["sync", "resume", &name], Duration::from_secs(45));
        resumed = matches!(&resume, Ok(o) if o.success());
        evidence.push(format!("resume: {resume:?}"));
        if resumed {
            std::fs::write(alpha.join("resume.txt"), "after-resume\n").expect("resume file");
            let mut after_resume = false;
            for _ in 0..20 {
                let _ = run_mutagen(&env, &["sync", "flush", &name], Duration::from_secs(20));
                let out = std::process::Command::new("docker")
                    .args(["exec", &container, "cat", "/workspace/resume.txt"])
                    .output()
                    .expect("docker exec");
                if String::from_utf8_lossy(&out.stdout).contains("after-resume") {
                    after_resume = true;
                    break;
                }
                std::thread::sleep(Duration::from_secs(1));
            }
            resumed = after_resume;
        }
        let _ = run_mutagen(&env, &["sync", "terminate", &name], Duration::from_secs(20));
        feeder.stop();
        let _ = run_mutagen(&env, &["daemon", "stop"], Duration::from_secs(15));
        Ok(())
    })();
    let local_preserved = alpha.join("alpha.txt").exists();
    let destroyed = destroy(&handle).await;
    let _ = std::fs::remove_dir_all(&alpha);
    assert!(destroyed.is_ok(), "destroy: {destroyed:?}");
    assert!(setup.is_ok(), "setup failed: {evidence:?}");
    assert!(synced_alpha, "alpha->beta not synced: {evidence:?}");
    assert!(synced_beta, "beta->alpha not synced: {evidence:?}");
    assert!(resumed, "resume failed: {evidence:?}");
    assert!(
        local_preserved,
        "local files must be preserved: {evidence:?}"
    );
}

#[tokio::test]
#[ignore = "live docker+mutagen: provisions a real sandbox, requires ssh-keyscan on PATH"]
async fn live_mutagen_wrong_password_fails_closed() {
    use pairproof::sandbox::lifecycle::{create_and_provision, destroy, destroy_by_session_id};
    use pairproof::sandbox::preflight::port_is_free;
    let port = (22700..22900u16)
        .find(|p| port_is_free(*p))
        .expect("free port");
    let session_id = format!("mutwrong{}x{}", std::process::id(), port);
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
    let alpha = std::env::temp_dir().join(format!(
        "pairproof-alpha-wrong-{}-{port}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&alpha);
    std::fs::create_dir_all(&alpha).expect("alpha");
    std::fs::write(alpha.join("alpha.txt"), "must-not-sync\n").expect("file");
    let name = format!("pairproof-mutagen-wrong-{}-{port}", std::process::id());
    let beta = pairproof::sync::beta_url(&endpoint, &username);
    let mut wrong = Secret::generate();
    while wrong == secret {
        wrong = Secret::generate();
    }
    let session = AskpassSession::create("mutagenwrong").expect("askpass");
    let mut failed_closed = false;
    let mut evidence = String::new();
    if let Ok(line) = verify_and_key_line(&endpoint, &fingerprint) {
        if let Ok(env) = MutagenAuthEnv::prepare(&session, &line) {
            let feeder = env.start_password_feeder(&wrong);
            let _ = run_mutagen(&env, &["daemon", "start"], Duration::from_secs(20));
            let created = run_mutagen(
                &env,
                &[
                    "sync",
                    "create",
                    "--name",
                    &name,
                    "--mode",
                    "two-way-safe",
                    "--symlink-mode",
                    "ignore",
                    &alpha.to_string_lossy(),
                    &beta,
                ],
                Duration::from_secs(50),
            );
            evidence = format!("create: {created:?}");
            std::thread::sleep(Duration::from_secs(2));
            let list = run_mutagen(&env, &["sync", "list"], Duration::from_secs(15));
            let not_watching = list.is_ok_and(|o| !o.stdout.contains("Watching"));
            let container = handle.container_name().to_string();
            let out = std::process::Command::new("docker")
                .args(["exec", &container, "cat", "/workspace/alpha.txt"])
                .output()
                .expect("docker exec");
            failed_closed =
                !String::from_utf8_lossy(&out.stdout).contains("must-not-sync") && not_watching;
            let _ = run_mutagen(&env, &["sync", "terminate", &name], Duration::from_secs(15));
            feeder.stop();
            let _ = run_mutagen(&env, &["daemon", "stop"], Duration::from_secs(15));
        }
    }
    let destroyed = destroy(&handle).await;
    let _ = std::fs::remove_dir_all(&alpha);
    assert!(destroyed.is_ok(), "destroy: {destroyed:?}");
    assert!(failed_closed, "wrong password must not sync: {evidence}");
}

#[tokio::test]
#[ignore = "live docker+mutagen: provisions a real sandbox, requires ssh-keyscan on PATH"]
async fn live_mutagen_wrong_host_key_fails_closed() {
    use pairproof::sandbox::lifecycle::{create_and_provision, destroy, destroy_by_session_id};
    use pairproof::sandbox::preflight::port_is_free;
    let port = (22900..23100u16)
        .find(|p| port_is_free(*p))
        .expect("free port");
    let session_id = format!("muthost{}x{}", std::process::id(), port);
    let endpoint = Endpoint::new("127.0.0.1", port).expect("endpoint");
    let username = EphemeralUsername::generate();
    let secret = Secret::generate();
    let (handle, _fingerprint) =
        match create_and_provision(&session_id, &endpoint, &username, &secret).await {
            Ok(pair) => pair,
            Err(err) => {
                let _ = destroy_by_session_id(&session_id).await;
                panic!("create_and_provision failed: {err}");
            }
        };
    let wrong_fp = pairproof::connection::Fingerprint::parse(&"b".repeat(64)).expect("fp");
    let mismatch = verify_and_key_line(&endpoint, &wrong_fp);
    let alpha = std::env::temp_dir().join(format!(
        "pairproof-alpha-host-{}-{port}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&alpha);
    std::fs::create_dir_all(&alpha).expect("alpha");
    std::fs::write(alpha.join("alpha.txt"), "must-not-sync-host\n").expect("file");
    let name = format!("pairproof-mutagen-host-{}-{port}", std::process::id());
    let beta = pairproof::sync::beta_url(&endpoint, &username);
    let session = AskpassSession::create("mutagenhost").expect("askpass");
    let mut failed_closed = false;
    let mut evidence = String::new();
    // Seed known_hosts with a different host's key line: ssh must reject it.
    let wrong_line = format!(
        "[127.0.0.1]:{port} {}",
        WRONG_KEY_LINE
            .split_whitespace()
            .skip(1)
            .collect::<Vec<_>>()
            .join(" ")
    );
    if let Ok(env) = MutagenAuthEnv::prepare(&session, &wrong_line) {
        let feeder = env.start_password_feeder(&secret);
        let _ = run_mutagen(&env, &["daemon", "start"], Duration::from_secs(20));
        let created = run_mutagen(
            &env,
            &[
                "sync",
                "create",
                "--name",
                &name,
                "--mode",
                "two-way-safe",
                "--symlink-mode",
                "ignore",
                &alpha.to_string_lossy(),
                &beta,
            ],
            Duration::from_secs(50),
        );
        evidence = format!("create: {created:?} mismatch_gate: {mismatch:?}");
        std::thread::sleep(Duration::from_secs(2));
        let list = run_mutagen(&env, &["sync", "list"], Duration::from_secs(15));
        let not_watching = list.is_ok_and(|o| !o.stdout.contains("Watching"));
        let container = handle.container_name().to_string();
        let out = std::process::Command::new("docker")
            .args(["exec", &container, "cat", "/workspace/alpha.txt"])
            .output()
            .expect("docker exec");
        failed_closed =
            !String::from_utf8_lossy(&out.stdout).contains("must-not-sync-host") && not_watching;
        let _ = run_mutagen(&env, &["sync", "terminate", &name], Duration::from_secs(15));
        feeder.stop();
        let _ = run_mutagen(&env, &["daemon", "stop"], Duration::from_secs(15));
    }
    let destroyed = destroy(&handle).await;
    let _ = std::fs::remove_dir_all(&alpha);
    assert!(destroyed.is_ok(), "destroy: {destroyed:?}");
    assert_eq!(mismatch, Err(SshAuthError::HostKeyMismatch));
    assert!(failed_closed, "wrong host key must not sync: {evidence}");
}
