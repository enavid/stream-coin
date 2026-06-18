use assert_cmd::Command;
use tempfile::tempdir;

fn sc(dir: &tempfile::TempDir) -> Command {
    let config_path = dir.path().join("config.toml");
    let mut cmd = Command::cargo_bin("sc").unwrap();
    cmd.env("SC_CONFIG_PATH", config_path.to_str().unwrap());
    cmd
}

#[test]
fn sc_shows_help_and_exits_successfully() {
    Command::cargo_bin("sc")
        .unwrap()
        .arg("--help")
        .assert()
        .success();
}

#[test]
fn sc_auth_subcommand_shows_help() {
    Command::cargo_bin("sc")
        .unwrap()
        .args(["auth", "--help"])
        .assert()
        .success();
}

#[test]
fn sc_ticker_subcommand_shows_help() {
    Command::cargo_bin("sc")
        .unwrap()
        .args(["ticker", "--help"])
        .assert()
        .success();
}

#[test]
fn sc_config_subcommand_shows_help() {
    Command::cargo_bin("sc")
        .unwrap()
        .args(["config", "--help"])
        .assert()
        .success();
}

#[test]
fn sc_without_args_exits_with_error() {
    Command::cargo_bin("sc").unwrap().assert().failure();
}

#[test]
fn sc_config_show_exits_successfully() {
    let dir = tempdir().unwrap();
    sc(&dir).args(["config", "show"]).assert().success();
}

#[test]
fn sc_config_show_prints_default_localhost_url() {
    let dir = tempdir().unwrap();
    sc(&dir)
        .args(["config", "show"])
        .assert()
        .success()
        .stdout(predicates::str::contains("localhost:8080"));
}

#[test]
fn sc_config_set_url_exits_successfully() {
    let dir = tempdir().unwrap();
    sc(&dir)
        .args(["config", "set-url", "http://prod.example.com"])
        .assert()
        .success();
}

#[test]
fn sc_config_set_url_persists_and_show_reflects_it() {
    let dir = tempdir().unwrap();

    sc(&dir)
        .args(["config", "set-url", "http://my-server.io:9000"])
        .assert()
        .success();

    sc(&dir)
        .args(["config", "show"])
        .assert()
        .success()
        .stdout(predicates::str::contains("my-server.io:9000"));
}

#[test]
fn sc_auth_status_exits_successfully() {
    let dir = tempdir().unwrap();
    sc(&dir).args(["auth", "status"]).assert().success();
}

#[test]
fn sc_auth_status_shows_not_authenticated_by_default() {
    let dir = tempdir().unwrap();
    sc(&dir)
        .args(["auth", "status"])
        .assert()
        .success()
        .stdout(predicates::str::contains("Not authenticated"));
}

#[test]
fn sc_auth_logout_on_unauthenticated_config_succeeds() {
    let dir = tempdir().unwrap();
    sc(&dir).args(["auth", "logout"]).assert().success();
}
