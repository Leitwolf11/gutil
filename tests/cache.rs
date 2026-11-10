#![allow(clippy::unwrap_used)]

use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use tempfile::tempdir;

const BLOB: &[u8] = b"hello";
const KEY: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

fn command(dir: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_gutil"));
    command
        .current_dir(dir)
        .env("HOME", dir)
        .env("USERPROFILE", dir)
        .env("GUTIL_CACHE_TOKEN", "must-not-send-over-http")
        .args(args);
    command
}

fn run(dir: &Path, args: &[&str]) -> Output {
    command(dir, args).output().unwrap()
}

fn errors(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn success(output: &Output) {
    assert!(output.status.success(), "{}", errors(output));
}

fn cache_entry(dir: &Path) -> std::path::PathBuf {
    dir.join(".gutil").join("cache").join("sha256").join(KEY)
}

#[test]
fn local_cache_uses_content_and_handles_concurrent_writers() {
    let dir = tempdir().unwrap();
    let path = dir.path();
    fs::write(path.join("-file"), BLOB).unwrap();
    let mut first = command(path, &["cache", "put", "--", "-file"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let second = run(path, &["cache", "put", "--", "-file"]);
    success(&second);
    let mut first_digest = String::new();
    first
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut first_digest)
        .unwrap();
    assert!(first.wait().unwrap().success());
    assert_eq!(first_digest, format!("{KEY}\n"));
    assert_eq!(second.stdout, first_digest.as_bytes());
    assert_eq!(
        fs::read_dir(cache_entry(path).parent().unwrap())
            .unwrap()
            .count(),
        1
    );
    fs::remove_file(path.join("-file")).unwrap();

    let output = run(path, &["cache", "--offline", "get", &KEY.to_uppercase()]);
    success(&output);
    assert_eq!(output.stdout, BLOB);
    assert_eq!(output.stderr, b"");

    fs::write(cache_entry(path), b"corrupt").unwrap();
    let output = run(path, &["cache", "get", KEY]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stdout, b"");
    assert!(errors(&output).contains("does not match"));
}

#[test]
fn local_config_uses_config_directory_and_profiles() {
    let dir = tempdir().unwrap();
    let path = dir.path();
    fs::create_dir(path.join("config")).unwrap();
    fs::write(path.join("file"), BLOB).unwrap();
    fs::write(
        path.join("config/team.toml"),
        r#"
[cache]
max-entry-bytes = 5
[cache.local]
directory = "artifacts"
[profiles.disabled.cache.local]
enabled = false
[profiles.tiny.cache]
max-entry-bytes = 4
"#,
    )
    .unwrap();
    let base = ["--config", "config/team.toml"];
    let put = run(path, &[base.as_slice(), &["cache", "put", "file"]].concat());
    success(&put);
    assert_eq!(
        fs::read(path.join("config/artifacts/sha256").join(KEY)).unwrap(),
        BLOB
    );
    for (args, message) in [
        (
            ["--profile", "disabled", "cache", "put", "file"],
            "no writable cache",
        ),
        (
            ["--profile", "tiny", "cache", "put", "file"],
            "4-byte cache limit",
        ),
        (
            ["--profile", "tiny", "cache", "get", KEY],
            "4-byte cache limit",
        ),
    ] {
        let output = run(path, &[base.as_slice(), &args].concat());
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(output.stdout, b"");
        assert!(errors(&output).contains(message));
    }
}

#[test]
fn reports_usage_missing_entries_and_invalid_configuration() {
    let dir = tempdir().unwrap();
    let path = dir.path();
    for args in [
        &["cache"][..],
        &["cache", "get", "../secret"],
        &["cache", "put", "a", "b"],
    ] {
        assert_eq!(run(path, args).status.code(), Some(2));
    }
    let missing = run(path, &["cache", "get", KEY]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(errors(&missing).contains("unavailable"));
    let directory = run(path, &["cache", "put", "."]);
    assert_eq!(directory.status.code(), Some(1));
    assert!(errors(&directory).contains("regular file"));

    for (config, message) in [
        ("[cache]\nmax-entry-bytes = 0", "max-entry-bytes"),
        ("[cache.remote]\ntimeout-ms = 0", "timeout-ms"),
        ("[cache.remote]\nmode = 'read-only'", "endpoint is required"),
        (
            "[cache.remote]\nendpoint = 'not-a-url'",
            "invalid cache.remote.endpoint",
        ),
        (
            "[cache.remote]\nendpoint = 'http://example.com'",
            "must use HTTPS",
        ),
        (
            "[cache.remote]\nendpoint = 'https://user:secret@example.com'",
            "omit credentials",
        ),
        (
            "[cache.remote]\nendpoint = 'https://example.com/?auth=secret'",
            "omit credentials",
        ),
    ] {
        fs::write(path.join("gutil.toml"), config).unwrap();
        let output = run(path, &["cache", "get", KEY]);
        assert_eq!(output.status.code(), Some(2));
        assert!(errors(&output).contains(message));
        let help = run(path, &["cache", "--help"]);
        success(&help);
        assert!(String::from_utf8_lossy(&help.stdout).contains("gutil cache"));
        let version = run(path, &["cache", "--version"]);
        success(&version);
        assert_eq!(
            version.stdout,
            format!("gutil {}\n", env!("CARGO_PKG_VERSION")).as_bytes()
        );
    }
}

fn server(dir: &Path, mode: &str, local_enabled: bool, extra: &str) -> TcpListener {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    fs::write(
        dir.join("gutil.toml"),
        format!(
            "{extra}\n[cache.local]\nenabled = {local_enabled}\n[cache.remote]\nmode = '{mode}'\nendpoint = 'http://{address}/team/'\n"
        ),
    )
    .unwrap();
    listener
}

fn respond(
    listener: &TcpListener,
    method: &'static str,
    response: &'static [u8],
    delay: Duration,
) -> JoinHandle<()> {
    let listener = listener.try_clone().unwrap();
    thread::spawn(move || {
        let started = Instant::now();
        let socket = loop {
            match listener.accept() {
                Ok((socket, _)) => break socket,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    assert!(
                        started.elapsed() < Duration::from_secs(5),
                        "no request received"
                    );
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("{error}"),
            }
        };
        socket.set_nonblocking(false).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut reader = BufReader::new(socket);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line, format!("{method} /team/sha256/{KEY} HTTP/1.1\r\n"));
        let mut length = 0;
        let mut binary = false;
        loop {
            line.clear();
            assert!(reader.read_line(&mut line).unwrap() > 0);
            if line == "\r\n" {
                break;
            }
            let (name, value) = line.trim_end().split_once(':').unwrap();
            assert!(!name.eq_ignore_ascii_case("authorization"));
            if name.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().unwrap();
            }
            if name.eq_ignore_ascii_case("content-type") {
                binary = value.trim() == "application/octet-stream";
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        if method == "PUT" {
            assert!(binary);
            assert_eq!(body, BLOB);
        } else {
            assert_eq!(body, b"");
        }
        if delay.is_zero() {
            reader.get_mut().write_all(response).unwrap();
        } else {
            let body_start = response
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .unwrap()
                + 4;
            let (head, body) = response.split_at(body_start);
            reader.get_mut().write_all(head).unwrap();
            thread::sleep(delay);
            let _ = reader.get_mut().write_all(body);
        }
    })
}

fn explicit(dir: &Path, args: &[&str]) -> Output {
    run(
        dir,
        &[&["--config", "gutil.toml", "cache"][..], args].concat(),
    )
}

fn no_request(listener: &TcpListener) {
    assert!(matches!(listener.accept(), Err(error) if error.kind() == io::ErrorKind::WouldBlock));
}

#[test]
fn remote_cache_writes_reads_and_repairs_local_entries() {
    let dir = tempdir().unwrap();
    let path = dir.path();
    let listener = server(path, "read-write", true, "");
    fs::write(path.join("file"), BLOB).unwrap();
    let put = respond(
        &listener,
        "PUT",
        b"HTTP/1.1 204 No Content\r\n\r\n",
        Duration::ZERO,
    );
    let output = explicit(path, &["put", "file"]);
    success(&output);
    assert_eq!(output.stdout, format!("{KEY}\n").as_bytes());
    put.join().unwrap();
    assert_eq!(fs::read(cache_entry(path)).unwrap(), BLOB);
    fs::write(cache_entry(path), b"corrupt").unwrap();

    let get = respond(
        &listener,
        "GET",
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello",
        Duration::ZERO,
    );
    let output = explicit(path, &["get", KEY]);
    success(&output);
    get.join().unwrap();
    assert_eq!(output.stdout, BLOB);
    assert!(errors(&output).contains("local cache read failed"));
    assert_eq!(fs::read(cache_entry(path)).unwrap(), BLOB);
    let hit = explicit(path, &["get", KEY]);
    success(&hit);
    assert_eq!(hit.stdout, BLOB);
    assert_eq!(hit.stderr, b"");
    no_request(&listener);
}

#[test]
fn remote_cache_can_store_with_local_disabled() {
    let dir = tempdir().unwrap();
    let path = dir.path();
    let listener = server(path, "read-write", false, "");
    fs::write(path.join("file"), BLOB).unwrap();
    let put = respond(
        &listener,
        "PUT",
        b"HTTP/1.1 201 Created\r\nContent-Length: 0\r\n\r\n",
        Duration::ZERO,
    );
    success(&explicit(path, &["put", "file"]));
    put.join().unwrap();
    assert!(!cache_entry(path).exists());

    let failed = respond(
        &listener,
        "PUT",
        b"HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\n\r\n",
        Duration::ZERO,
    );
    let output = explicit(path, &["put", "file"]);
    failed.join().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(errors(&output).contains("PUT returned HTTP 503"));
    assert_eq!(output.stdout, b"");
}

#[test]
fn remote_cache_respects_read_only_offline_and_project_trust() {
    let dir = tempdir().unwrap();
    let path = dir.path();
    let listener = server(path, "read-only", true, "");
    fs::write(path.join("file"), BLOB).unwrap();
    success(&explicit(path, &["put", "file"]));
    fs::remove_file(cache_entry(path)).unwrap();
    let offline = explicit(path, &["--offline", "get", KEY]);
    assert_eq!(offline.status.code(), Some(1));
    assert!(!errors(&offline).contains("remote cache"));
    let discovered = run(path, &["cache", "get", KEY]);
    assert_eq!(discovered.status.code(), Some(1));
    assert!(errors(&discovered).contains("select it with --config"));
    no_request(&listener);

    let get = respond(
        &listener,
        "GET",
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello",
        Duration::ZERO,
    );
    let home = path.join(".gutil");
    fs::rename(path.join("gutil.toml"), home.join("config.toml")).unwrap();
    let output = run(path, &["cache", "get", KEY]);
    success(&output);
    assert_eq!(output.stdout, BLOB);
    get.join().unwrap();
}

#[test]
fn remote_cache_rejects_corruption_oversized_bodies_redirects_and_misses() {
    let dir = tempdir().unwrap();
    let path = dir.path();
    let listener = server(path, "read-only", true, "[cache]\nmax-entry-bytes = 5");
    let responses: &[(&[u8], &str)] = &[
        (b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n", "unavailable"),
        (b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nwrong", "does not match"),
        (b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nhello!", "5-byte cache limit"),
        (b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nhel\r\n3\r\nlo!\r\n0\r\n\r\n", "5-byte cache limit"),
        (b"HTTP/1.1 302 Found\r\nLocation: /redirect\r\nContent-Length: 0\r\n\r\n", "GET returned HTTP 302"),
    ];
    for (response, diagnostic) in responses {
        let get = respond(&listener, "GET", response, Duration::ZERO);
        let output = explicit(path, &["get", KEY]);
        get.join().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(output.stdout, b"");
        assert!(errors(&output).contains(diagnostic), "{}", errors(&output));
        assert!(!cache_entry(path).exists());
    }
    no_request(&listener);
}

#[test]
fn backend_failures_preserve_a_successful_local_put_or_remote_get() {
    let dir = tempdir().unwrap();
    let path = dir.path();
    let listener = server(path, "read-write", true, "");
    fs::write(path.join("file"), BLOB).unwrap();
    let put = respond(
        &listener,
        "PUT",
        b"HTTP/1.1 500 Error\r\nContent-Length: 0\r\n\r\n",
        Duration::ZERO,
    );
    let output = explicit(path, &["put", "file"]);
    success(&output);
    put.join().unwrap();
    assert!(errors(&output).contains("PUT returned HTTP 500"));

    fs::remove_dir_all(path.join(".gutil")).unwrap();
    fs::write(path.join(".gutil"), b"blocks cache directory").unwrap();
    let get = respond(
        &listener,
        "GET",
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello",
        Duration::ZERO,
    );
    let output = explicit(path, &["get", KEY]);
    success(&output);
    get.join().unwrap();
    assert_eq!(output.stdout, BLOB);
    assert!(errors(&output).contains("local cache write failed"));
}

#[test]
fn remote_cache_times_out_while_reading_the_response() {
    let dir = tempdir().unwrap();
    let path = dir.path();
    let listener = server(path, "read-only", false, "");
    let config = path.join("gutil.toml");
    let mut contents = fs::read_to_string(&config).unwrap();
    contents.push_str("timeout-ms = 500\n");
    fs::write(config, contents).unwrap();
    let get = respond(
        &listener,
        "GET",
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello",
        Duration::from_millis(750),
    );
    let output = explicit(path, &["get", KEY]);
    get.join().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stdout, b"");
    assert!(errors(&output).contains("timed out"));
}

#[test]
fn reports_missing_home_for_default_local_directory() {
    let dir = tempdir().unwrap();
    let output = command(dir.path(), &["cache", "get", KEY])
        .env_remove("HOME")
        .env_remove("USERPROFILE")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(errors(&output).contains("local cache has no home directory"));
}

#[test]
fn https_cache_rejects_invalid_bearer_tokens_without_contacting_the_service() {
    let dir = tempdir().unwrap();
    let path = dir.path();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    fs::write(
        path.join("gutil.toml"),
        format!("[cache.remote]\nmode = 'read-only'\nendpoint = 'https://{address}'\n"),
    )
    .unwrap();
    let output = command(path, &["--config", "gutil.toml", "cache", "get", KEY])
        .env("GUTIL_CACHE_TOKEN", "private\ninvalid")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stdout, b"");
    assert!(errors(&output).contains("remote cache request failed"));
    assert!(!errors(&output).contains("private"));
    no_request(&listener);
}
