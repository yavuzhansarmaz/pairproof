use pairproof::connection::{Endpoint, EphemeralUsername};
use pairproof::sandbox::command::{
    container_remove, container_run, docker_info, docker_version, exec_as, exec_root, image_build,
    image_inspect, image_tag, network_create, network_remove,
};
use pairproof::sandbox::parse::{
    is_path_within_workspace, parse_docker_server_version, parse_docker_version,
    parse_host_key_fingerprint, parse_realpath,
};

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| a.to_string()).collect()
}

#[test]
fn container_run_argv_exact_hardening() {
    let endpoint = Endpoint::new("203.0.113.10", 2222).expect("endpoint");
    let spec = container_run("s1", &endpoint, &image_tag()).expect("run spec");
    assert_eq!(spec.program, "docker");
    assert_eq!(
        spec.args,
        strings(&[
            "run",
            "-d",
            "--name",
            "pairproof-s1",
            "--network",
            "pairproof-net-s1",
            "--memory",
            "512m",
            "--cpus",
            "1",
            "--pids-limit",
            "256",
            "--cap-drop",
            "NET_RAW",
            "--cap-add",
            "NET_ADMIN",
            "-p",
            "0.0.0.0:2222:22",
            &image_tag(),
        ])
    );
    for forbidden in ["-v", "--mount", "--privileged", "-e", "--volume"] {
        assert!(
            !spec.args.iter().any(|a| a == forbidden),
            "argv must not contain {forbidden}"
        );
    }
    assert_eq!(spec.args.iter().filter(|a| *a == "-p").count(), 1);
}

#[test]
fn exec_builders_carry_user_and_argv_verbatim() {
    let user = EphemeralUsername::parse("pp-abcd1234").expect("username");
    let argv = strings(&[
        "python3",
        "-m",
        "py_compile",
        "/workspace/questions/01/solution.py",
    ]);
    let spec = exec_as("pairproof-s1", &user, &argv, false).expect("exec spec");
    assert_eq!(
        spec.args,
        strings(&[
            "exec",
            "-u",
            "pp-abcd1234",
            "pairproof-s1",
            "python3",
            "-m",
            "py_compile",
            "/workspace/questions/01/solution.py",
        ])
    );
    let interactive = exec_as("pairproof-s1", &user, &strings(&["cat", "--", "f"]), true)
        .expect("interactive exec");
    assert_eq!(
        interactive.args,
        strings(&[
            "exec",
            "-i",
            "-u",
            "pp-abcd1234",
            "pairproof-s1",
            "cat",
            "--",
            "f"
        ])
    );
    let root = exec_root("pairproof-s1", &strings(&["chpasswd"]), true).expect("root exec");
    assert_eq!(
        root.args,
        strings(&["exec", "-i", "pairproof-s1", "chpasswd"])
    );
    assert!(exec_root("pairproof-net-s1", &strings(&["true"]), false).is_err());
    assert!(exec_root("pairproof-s1", &[], false).is_err());
    assert!(exec_root("pairproof-s1", &strings(&["-e", "X=1"]), false).is_err());
    assert!(exec_root("pairproof-s1", &strings(&["bad\u{0}arg"]), false).is_err());
    assert!(exec_as("random-name", &user, &strings(&["true"]), false).is_err());
}

#[test]
fn builders_reject_invalid_session_ids() {
    let endpoint = Endpoint::new("203.0.113.10", 2222).expect("endpoint");
    for bad in ["--privileged", "a/b", "x;rm", "", "a b"] {
        assert!(
            container_run(bad, &endpoint, &image_tag()).is_err(),
            "run {bad:?}"
        );
        assert!(container_remove(bad).is_err(), "rm {bad:?}");
        assert!(network_create(bad).is_err(), "net create {bad:?}");
        assert!(network_remove(bad).is_err(), "net rm {bad:?}");
    }
    assert!(container_run("s1", &endpoint, "--privileged").is_err());
    assert!(container_run("s1", &endpoint, "").is_err());
}

#[test]
fn probe_and_lifecycle_builders_argv() {
    assert_eq!(docker_version().args, strings(&["--version"]));
    assert_eq!(docker_version().program, "docker");
    assert_eq!(
        docker_info().args,
        strings(&["info", "--format", "{{.ServerVersion}}"])
    );
    assert_eq!(
        image_inspect("img:1").expect("inspect").args,
        strings(&["image", "inspect", "img:1"])
    );
    assert_eq!(
        image_build("img:1", "/tmp/ctx").expect("build").args,
        strings(&["build", "-t", "img:1", "/tmp/ctx"])
    );
    assert_eq!(
        network_create("s1").expect("net create").args,
        strings(&["network", "create", "pairproof-net-s1"])
    );
    assert_eq!(
        network_remove("s1").expect("net rm").args,
        strings(&["network", "rm", "pairproof-net-s1"])
    );
    assert_eq!(
        container_remove("s1").expect("rm").args,
        strings(&["rm", "-f", "pairproof-s1"])
    );
}

#[test]
fn network_create_is_plain_bridge() {
    let spec = network_create("s1").expect("net create");
    assert_eq!(
        spec.args,
        strings(&["network", "create", "pairproof-net-s1"])
    );
    assert!(!spec.args.iter().any(|a| a == "--internal"));
}

#[test]
fn image_tag_includes_crate_version() {
    assert_eq!(
        image_tag(),
        format!("pairproof-sandbox:{}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn parse_docker_version_fixture() {
    assert_eq!(
        parse_docker_version("Docker version 29.9.2, build 8105a37\n").expect("parse"),
        "29.9.2"
    );
    assert!(parse_docker_version("").is_err());
    assert!(parse_docker_version("podman version 5.0\n").is_err());
    assert_eq!(
        parse_docker_server_version(" 29.9.2 \n").expect("server version"),
        "29.9.2"
    );
    assert!(parse_docker_server_version("").is_err());
    assert!(parse_docker_server_version("29.9.2 extra").is_err());
}

#[test]
fn parse_host_key_fingerprint_known_vector_and_rejects_garbage() {
    let line = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOr8qXRxd07w1+T+R1EJccWCxO7Z82oSjs0+Knb0KoAA test@example";
    let parsed = parse_host_key_fingerprint(line).expect("fingerprint");
    assert_eq!(
        parsed.as_str(),
        "d427b935052747c277d2b09d8d6c39ded174e244b2d032c9032b1c994b0689e4"
    );
    assert!(parse_host_key_fingerprint("").is_err());
    assert!(parse_host_key_fingerprint("ssh-ed25519 #### comment").is_err());
    assert!(parse_host_key_fingerprint("ssh-ed25519 TWF comment").is_err());
}

#[test]
fn parse_realpath_and_workspace_prefix() {
    assert_eq!(
        parse_realpath("/workspace/questions/01/solution.py\n").expect("realpath"),
        "/workspace/questions/01/solution.py"
    );
    assert!(parse_realpath("relative/path\n").is_err());
    assert!(parse_realpath("").is_err());
    assert!(parse_realpath("/workspace/a\n/etc/passwd\n").is_err());
    assert!(is_path_within_workspace("/workspace"));
    assert!(is_path_within_workspace(
        "/workspace/questions/01/solution.py"
    ));
    assert!(!is_path_within_workspace("/workspace-evil/x"));
    assert!(!is_path_within_workspace("/etc/passwd"));
}

#[test]
fn image_builders_reject_flaglike_tag_and_context() {
    assert!(image_inspect("--format={{.Id}}").is_err());
    assert!(image_inspect("").is_err());
    assert!(image_inspect("-q").is_err());
    assert!(image_build("--format={{.Id}}", "/tmp/ctx").is_err());
    assert!(image_build("", "/tmp/ctx").is_err());
    assert!(image_build(&image_tag(), "/tmp/a b").is_err());
    assert!(image_build(&image_tag(), "ctx\twith\ttabs").is_err());
    assert!(image_build(&image_tag(), "--pull").is_err());
    assert!(image_build(&image_tag(), "").is_err());
    let tag = image_tag();
    assert_eq!(
        image_inspect(&tag).expect("inspect").args,
        strings(&["image", "inspect", &tag])
    );
    assert_eq!(
        image_build(&tag, "/tmp/ctx").expect("build").args,
        strings(&["build", "-t", &tag, "/tmp/ctx"])
    );
}
