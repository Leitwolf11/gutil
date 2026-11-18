#![allow(clippy::unwrap_used)]

use std::ffi::{OsStr, OsString};
use std::process::{Command, Stdio};

use proptest::prelude::*;

use super::{valid_tag, GitHubSource};

fn arguments(source: &str) -> Vec<OsString> {
    let source = GitHubSource::parse(OsStr::new(source)).unwrap();
    let mut command = Command::new("cargo");
    source.configure_cargo(&mut command);
    command.get_args().map(OsStr::to_owned).collect()
}

#[test]
fn selects_tags_and_commits_without_changing_the_repository_url() {
    let prefix = ["--git", "https://github.com/owner/repo"];
    assert_eq!(arguments("github.com:owner/repo"), prefix);
    for (selector, option) in [
        ("v4", "--tag=v4"),
        ("release/v4", "--tag=release/v4"),
        ("-v4", "--tag=-v4"),
        ("v4@stable", "--tag=v4@stable"),
        ("v4.LOCK", "--tag=v4.LOCK"),
        (
            "--config=net.offline=true",
            "--tag=--config=net.offline=true",
        ),
        ("abcdef0", "--rev=abcdef0"),
        (
            "ABCDEF0123456789ABCDEF0123456789ABCDEF01",
            "--rev=abcdef0123456789abcdef0123456789abcdef01",
        ),
    ] {
        assert_eq!(
            arguments(&format!("github.com:owner/repo@{selector}")),
            [prefix[0], prefix[1], option],
        );
    }
}

#[test]
fn rejects_invalid_selectors_and_repository_names() {
    for source in [
        "github.com:owner/repo@",
        "github.com:owner/repo@v4..old",
        "github.com:owner/repo@v4.lock",
        "github.com:owner/repo@release/.v4",
        "github.com:owner/repo@v4/",
        "github.com:owner/repo@v4//next",
        "github.com:owner/repo@v4@{1}",
        "github.com:owner/repo@v4^",
        "github.com:owner/repo@v4~1",
        "github.com:owner/repo@v4?",
        "github.com:owner/repo@bad tag",
        "github.com:owner/repo@v4\0",
        "github.com:owner/repo/extra@v4",
        "github.com:owner/..@v4",
        "github.com:/repo@v4",
        "github.com:owner/@v4",
    ] {
        assert!(
            GitHubSource::parse(OsStr::new(source)).is_err(),
            "{source:?}"
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn tag_validation_matches_git(tag in "[ -~]{0,40}") {
        let result = Command::new("git")
            .args(["check-ref-format", &format!("refs/tags/{tag}")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        prop_assert_eq!(valid_tag(&tag), result.success());
    }
}
