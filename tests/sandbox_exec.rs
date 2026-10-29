use std::time::Duration;

use pairproof::connection::{Endpoint, EphemeralUsername, Secret};
use pairproof::core::Language;
use pairproof::sandbox::error::SandboxError;
use pairproof::sandbox::exec::{
    check_runnability, run_in_container, run_tests, runnability_argv, test_argv,
};
use pairproof::sandbox::lifecycle::{
    create_and_provision, destroy, destroy_by_session_id, SandboxHandle,
};
use pairproof::sandbox::validate::WorkspacePath;
use pairproof::sandbox::workspace::{read_file, write_file};

fn handle() -> SandboxHandle {
    let endpoint = Endpoint::new("127.0.0.1", 2222).expect("endpoint");
    SandboxHandle::ids("ex1", &endpoint).expect("handle")
}

fn user() -> EphemeralUsername {
    EphemeralUsername::parse("pp-abcd1234").expect("user")
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| a.to_string()).collect()
}

#[tokio::test]
async fn run_in_container_rejects_non_allowlist_before_docker() {
    let timeout = Duration::from_secs(5);
    let cases: Vec<Vec<String>> = vec![
        strings(&["sh", "-c", "x"]),
        strings(&["-e"]),
        vec![],
        strings(&["python3;rm"]),
        vec!["python3".to_string(), "a\0b".to_string()],
    ];
    for argv in cases {
        let err = run_in_container(&handle(), &user(), &argv, None, timeout)
            .await
            .expect_err("argv must be rejected");
        assert!(
            matches!(err, SandboxError::InvalidCommand(_)),
            "argv {argv:?} gave {err:?}"
        );
    }
}

#[test]
fn runnability_argv_exact_shapes() {
    let py = WorkspacePath::new("questions/01/solution.py").expect("path");
    assert_eq!(
        runnability_argv(Language::Python, &py),
        strings(&[
            "python3",
            "-m",
            "py_compile",
            "/workspace/questions/01/solution.py",
        ])
    );
    let js = WorkspacePath::new("questions/02/solution.js").expect("path");
    assert_eq!(
        runnability_argv(Language::JavaScript, &js),
        strings(&["node", "--check", "/workspace/questions/02/solution.js"])
    );
    let sql = WorkspacePath::new("questions/03/query.sql").expect("path");
    assert!(runnability_argv(Language::Sql, &sql).is_empty());
}

#[test]
fn test_argv_exact_shapes() {
    let py = WorkspacePath::new("questions/01/solution.py").expect("path");
    assert_eq!(
        test_argv(Language::Python, &py),
        strings(&["python3", "/workspace/questions/01/solution.py"])
    );
    let js = WorkspacePath::new("questions/02/solution.js").expect("path");
    assert_eq!(
        test_argv(Language::JavaScript, &js),
        strings(&["node", "/workspace/questions/02/solution.js"])
    );
    let sql = WorkspacePath::new("questions/03/query.sql").expect("path");
    assert!(test_argv(Language::Sql, &sql).is_empty());
}

fn live_session_id(tag: &str) -> String {
    format!("xe{}x{tag}", std::process::id())
}

fn free_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port()
}

async fn live_handle(session_id: &str) -> (SandboxHandle, EphemeralUsername) {
    let endpoint = Endpoint::new("127.0.0.1", free_port()).expect("endpoint");
    let user = EphemeralUsername::generate();
    let secret = Secret::generate();
    match create_and_provision(session_id, &endpoint, &user, &secret).await {
        Ok((handle, _)) => (handle, user),
        Err(err) => {
            let _ = destroy_by_session_id(session_id).await;
            panic!("create_and_provision failed: {err}");
        }
    }
}

fn timeout() -> Duration {
    Duration::from_secs(30)
}

#[tokio::test]
#[ignore = "live docker: python runnability pass and fail in own container"]
async fn live_runnability_python_pass_fail() {
    let session_id = live_session_id("p");
    let (handle, user) = live_handle(&session_id).await;
    let outcome = async {
        let good = WorkspacePath::new("runnable/good.py").expect("path");
        write_file(&handle, &user, &good, b"print('ok')\n")
            .await
            .map_err(|e| e.to_string())?;
        let pass = check_runnability(&handle, &user, &good, Language::Python, timeout())
            .await
            .map_err(|e| e.to_string())?;
        if !pass.runnable || pass.exit_code != Some(0) {
            return Err(format!("python pass misreported: {pass:?}"));
        }
        let bad = WorkspacePath::new("runnable/bad.py").expect("path");
        write_file(&handle, &user, &bad, b"def f(:\n")
            .await
            .map_err(|e| e.to_string())?;
        let fail = check_runnability(&handle, &user, &bad, Language::Python, timeout())
            .await
            .map_err(|e| e.to_string())?;
        if fail.runnable {
            return Err(format!("python fail misreported: {fail:?}"));
        }
        Ok(())
    }
    .await;
    let _ = destroy(&handle).await;
    assert!(outcome.is_ok(), "{outcome:?}");
}

#[tokio::test]
#[ignore = "live docker: node runnability pass and fail in own container"]
async fn live_runnability_js_pass_fail() {
    let session_id = live_session_id("j");
    let (handle, user) = live_handle(&session_id).await;
    let outcome = async {
        let good = WorkspacePath::new("runnable/good.js").expect("path");
        write_file(&handle, &user, &good, b"console.log('ok');\n")
            .await
            .map_err(|e| e.to_string())?;
        let pass = check_runnability(&handle, &user, &good, Language::JavaScript, timeout())
            .await
            .map_err(|e| e.to_string())?;
        if !pass.runnable || pass.exit_code != Some(0) {
            return Err(format!("js pass misreported: {pass:?}"));
        }
        let bad = WorkspacePath::new("runnable/bad.js").expect("path");
        write_file(&handle, &user, &bad, b"function ({\n")
            .await
            .map_err(|e| e.to_string())?;
        let fail = check_runnability(&handle, &user, &bad, Language::JavaScript, timeout())
            .await
            .map_err(|e| e.to_string())?;
        if fail.runnable {
            return Err(format!("js fail misreported: {fail:?}"));
        }
        Ok(())
    }
    .await;
    let _ = destroy(&handle).await;
    assert!(outcome.is_ok(), "{outcome:?}");
}

#[tokio::test]
#[ignore = "live docker: sql runnability pass and fail in own container"]
async fn live_runnability_sql_pass_fail() {
    let session_id = live_session_id("s");
    let (handle, user) = live_handle(&session_id).await;
    let outcome = async {
        let good = WorkspacePath::new("runnable/good.sql").expect("path");
        write_file(&handle, &user, &good, b"SELECT 1;\n")
            .await
            .map_err(|e| e.to_string())?;
        let pass = check_runnability(&handle, &user, &good, Language::Sql, timeout())
            .await
            .map_err(|e| e.to_string())?;
        if !pass.runnable || pass.exit_code != Some(0) {
            return Err(format!("sql pass misreported: {pass:?}"));
        }
        let bad = WorkspacePath::new("runnable/bad.sql").expect("path");
        write_file(&handle, &user, &bad, b"SELEC 1 FRM nowhere;\n")
            .await
            .map_err(|e| e.to_string())?;
        let fail = check_runnability(&handle, &user, &bad, Language::Sql, timeout())
            .await
            .map_err(|e| e.to_string())?;
        if fail.runnable {
            return Err(format!("sql fail misreported: {fail:?}"));
        }
        Ok(())
    }
    .await;
    let _ = destroy(&handle).await;
    assert!(outcome.is_ok(), "{outcome:?}");
}

#[tokio::test]
#[ignore = "live docker: python test-run pass and fail in own container"]
async fn live_test_run_python_pass_fail() {
    let session_id = live_session_id("t");
    let (handle, user) = live_handle(&session_id).await;
    let outcome = async {
        let pass_path = WorkspacePath::new("tests/pass.py").expect("path");
        let pass_code = "def add(a, b):\n    return a + b\n\nif __name__ == '__main__':\n    assert add(1, 2) == 3\n";
        write_file(&handle, &user, &pass_path, pass_code.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        let pass = run_tests(&handle, &user, &pass_path, Language::Python, timeout())
            .await
            .map_err(|e| e.to_string())?;
        if !pass.success || pass.exit_code != Some(0) {
            return Err(format!("python test pass misreported: {pass:?}"));
        }
        let fail_path = WorkspacePath::new("tests/fail.py").expect("path");
        let fail_code = "if __name__ == '__main__':\n    assert 1 == 2\n";
        write_file(&handle, &user, &fail_path, fail_code.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        let fail = run_tests(&handle, &user, &fail_path, Language::Python, timeout())
            .await
            .map_err(|e| e.to_string())?;
        if fail.success {
            return Err(format!("python test fail misreported: {fail:?}"));
        }
        Ok(())
    }
    .await;
    let _ = destroy(&handle).await;
    assert!(outcome.is_ok(), "{outcome:?}");
}

#[tokio::test]
#[ignore = "live docker: js and sql test-runs incl SELECT in own container"]
async fn live_test_run_js_and_sql() {
    let session_id = live_session_id("m");
    let (handle, user) = live_handle(&session_id).await;
    let outcome = async {
        let js = WorkspacePath::new("tests/t.js").expect("path");
        write_file(
            &handle,
            &user,
            &js,
            b"if (1 + 1 !== 2) { throw new Error('bad'); }\nconsole.log('ok');\n",
        )
        .await
        .map_err(|e| e.to_string())?;
        let pass = run_tests(&handle, &user, &js, Language::JavaScript, timeout())
            .await
            .map_err(|e| e.to_string())?;
        if !pass.success {
            return Err(format!("js test pass misreported: {pass:?}"));
        }
        let js_fail = WorkspacePath::new("tests/f.js").expect("path");
        write_file(&handle, &user, &js_fail, b"throw new Error('boom');\n")
            .await
            .map_err(|e| e.to_string())?;
        let fail = run_tests(&handle, &user, &js_fail, Language::JavaScript, timeout())
            .await
            .map_err(|e| e.to_string())?;
        if fail.success {
            return Err(format!("js test fail misreported: {fail:?}"));
        }
        let sql = WorkspacePath::new("tests/q.sql").expect("path");
        write_file(&handle, &user, &sql, b"SELECT 1;\n")
            .await
            .map_err(|e| e.to_string())?;
        let select = run_tests(&handle, &user, &sql, Language::Sql, timeout())
            .await
            .map_err(|e| e.to_string())?;
        if !select.success {
            return Err(format!("sql SELECT misreported: {select:?}"));
        }
        let sql_fail = WorkspacePath::new("tests/b.sql").expect("path");
        write_file(
            &handle,
            &user,
            &sql_fail,
            b"SELECT * FROM no_such_table_xyz;\n",
        )
        .await
        .map_err(|e| e.to_string())?;
        let bad = run_tests(&handle, &user, &sql_fail, Language::Sql, timeout())
            .await
            .map_err(|e| e.to_string())?;
        if bad.success {
            return Err(format!("sql fail misreported: {bad:?}"));
        }
        Ok(())
    }
    .await;
    let _ = destroy(&handle).await;
    assert!(outcome.is_ok(), "{outcome:?}");
}

#[tokio::test]
#[ignore = "live docker: infinite loop must time out and die in own container"]
async fn live_run_timeout_kill() {
    let session_id = live_session_id("k");
    let (handle, user) = live_handle(&session_id).await;
    let outcome = async {
        let path = WorkspacePath::new("loop/loop.py").expect("path");
        let code = "import pathlib,time\np=pathlib.Path('/workspace/loop/heartbeat.txt')\nn=0\nwhile True:\n n+=1\n p.write_text(str(n))\n time.sleep(0.05)\n";
        write_file(&handle, &user, &path, code.as_bytes()).await.map_err(|e| e.to_string())?;
        let argv = vec!["python3".to_string(), "/workspace/loop/loop.py".to_string()];
        match run_in_container(&handle, &user, &argv, None, Duration::from_secs(1)).await {
            Err(SandboxError::Timeout { .. }) => {}
            Err(other) => return Err(format!("wrong error for infinite loop: {other}")),
            Ok(out) => return Err(format!("infinite loop must time out: {out:?}")),
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
        let hb = WorkspacePath::new("loop/heartbeat.txt").expect("path");
        let first = read_file(&handle, &user, &hb).await.map_err(|e| e.to_string())?;
        tokio::time::sleep(Duration::from_millis(400)).await;
        let second = read_file(&handle, &user, &hb).await.map_err(|e| e.to_string())?;
        if first.is_empty() || first != second {
            return Err(format!("loop survived timeout: {first:?} -> {second:?}"));
        }
        Ok(())
    }
    .await;
    let _ = destroy(&handle).await;
    assert!(outcome.is_ok(), "{outcome:?}");
}

#[tokio::test]
#[ignore = "live docker: oversized output must set truncation flags"]
async fn live_run_truncation() {
    use pairproof::sandbox::process::EXEC_MAX_OUTPUT;

    let session_id = live_session_id("c");
    let (handle, user) = live_handle(&session_id).await;
    let outcome = async {
        let argv = vec![
            "python3".to_string(),
            "-c".to_string(),
            "print('x' * 1000000)".to_string(),
        ];
        let out = run_in_container(&handle, &user, &argv, None, timeout())
            .await
            .map_err(|e| e.to_string())?;
        if !out.success() {
            return Err(format!("print must succeed: {out:?}"));
        }
        if !out.stdout_truncated || out.stdout.len() != EXEC_MAX_OUTPUT {
            return Err(format!(
                "truncation flags misreported: len={} truncated={}",
                out.stdout.len(),
                out.stdout_truncated
            ));
        }
        Ok(())
    }
    .await;
    let _ = destroy(&handle).await;
    assert!(outcome.is_ok(), "{outcome:?}");
}

#[tokio::test]
#[ignore = "live docker: sql runnability must not mutate session db"]
async fn live_sql_runnability_does_not_mutate_session_db() {
    let session_id = live_session_id("i");
    let (handle, user) = live_handle(&session_id).await;
    let outcome = async {
        let evil = WorkspacePath::new("runnable/evil.sql").expect("path");
        write_file(&handle, &user, &evil, b"CREATE TABLE IF NOT EXISTS pairproof.pwned (id INT); INSERT INTO pairproof.pwned VALUES (1);\n").await.map_err(|e| e.to_string())?;
        let report = check_runnability(&handle, &user, &evil, Language::Sql, timeout()).await.map_err(|e| e.to_string())?;
        if report.runnable {
            return Err(format!("evil sql must be denied: {report:?}"));
        }
        let check = strings(&["mariadb", "-N", "-e", "SELECT COUNT(*) FROM information_schema.TABLES WHERE TABLE_SCHEMA='pairproof' AND TABLE_NAME='pwned'"]);
        let out = run_in_container(&handle, &user, &check, None, timeout()).await.map_err(|e| e.to_string())?;
        if !out.success() || out.stdout_text().trim() != "0" {
            return Err(format!("pairproof mutated or check failed: {out:?}"));
        }
        Ok(())
    }
    .await;
    let _ = destroy(&handle).await;
    assert!(outcome.is_ok(), "{outcome:?}");
}
