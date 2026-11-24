use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::tempdir;

use super::{create_fixture, git, gutil, install_command, success};

fn commit(directory: &Path) -> String {
    git(directory, &["add", "."]);
    git(
        directory,
        &[
            "-c",
            "user.name=gutil test",
            "-c",
            "user.email=gutil@example.test",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "Update the pinned plug-in fixture",
        ],
    );
    let output = Command::new("git")
        .current_dir(directory)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    success(&output);
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn update_output(repository: &Path, prefix: &str) -> String {
    fs::write(
        repository.join("src/main.rs"),
        format!(
            "fn main() {{ println!(\"{prefix}: {{}}\", std::env::args().nth(1).unwrap()); }}\n"
        ),
    )
    .unwrap();
    commit(repository)
}

#[test]
fn installs_the_selected_tag_or_commit_and_never_falls_back_to_head() {
    let fixture = tempdir().unwrap();
    let repository = fixture.path();
    create_fixture(repository);
    git(
        repository,
        &[
            "-c",
            "user.name=gutil test",
            "-c",
            "user.email=gutil@example.test",
            "-c",
            "tag.gpgsign=false",
            "tag",
            "-a",
            "v4",
            "-m",
            "Version four",
        ],
    );
    let pinned = update_output(repository, "pinned");
    git(
        repository,
        &["-c", "tag.gpgsign=false", "tag", "releases/v4", "v4^{}"],
    );
    git(
        repository,
        &["-c", "tag.gpgsign=false", "tag", "20240101", "v4^{}"],
    );
    update_output(repository, "head");
    git(repository, &["branch", "v4"]);

    for (selector, expected) in [
        ("v4".to_owned(), Some("example\n")),
        ("releases/v4".to_owned(), Some("example\n")),
        ("refs/tags/20240101".to_owned(), Some("example\n")),
        (pinned[..8].to_owned(), Some("pinned: example\n")),
        (pinned, Some("pinned: example\n")),
        ("missing-tag".to_owned(), None),
        ("0000000000000000000000000000000000000000".to_owned(), None),
    ] {
        let home = tempdir().unwrap();
        let directory = home.path();
        let output = install_command(
            directory,
            repository,
            "https://github.com/pinned-owner/pinned-repo",
        )
        .args([
            "plug-in",
            "install",
            &format!("github.com:pinned-owner/pinned-repo@{selector}"),
        ])
        .output()
        .unwrap();
        if let Some(expected) = expected {
            success(&output);
            let launched = gutil(directory)
                .args(["plug-in", "use", "plugin-command", "example"])
                .output()
                .unwrap();
            success(&launched);
            assert_eq!(launched.stdout, expected.as_bytes());
        } else {
            assert_eq!(output.status.code(), Some(1));
            assert!(String::from_utf8_lossy(&output.stderr).contains("cargo install failed"));
            assert!(!directory
                .join(".gutil/plugins/gutil-plugin-command.wasm")
                .exists());
        }
    }
}
