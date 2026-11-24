#![allow(clippy::unwrap_used)]

use std::ffi::{OsStr, OsString};
use std::process::{Command, Stdio};

use proptest::prelude::*;

use super::{valid_tag, InstallSource, Registries};

fn arguments(source: &str) -> Vec<OsString> {
    let source = InstallSource::parse(OsStr::new(source), None, &Registries::default()).unwrap();
    let mut command = Command::new("cargo");
    source.configure_cargo(&mut command).unwrap();
    command.get_args().map(OsStr::to_owned).collect()
}

#[test]
fn selects_tags_and_commits_without_changing_the_repository_url() {
    let prefix = ["--git", "https://github.com/owner/repo"];
    assert_eq!(arguments("github.com:owner/repo"), prefix);
    for (selector, option) in [
        ("v4", "--tag=v4"),
        ("release/v4", "--tag=release/v4"),
        ("refs/tags/20240101", "--tag=20240101"),
        ("refs/tags/abcdef0", "--tag=abcdef0"),
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
        "github.com:owner/repo@refs/tags/",
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
            InstallSource::parse(OsStr::new(source), None, &Registries::default()).is_err(),
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

#[test]
fn supports_hosted_repositories_and_registry_packages() {
    assert_eq!(
        arguments("gitlab.com:group/sub_group/Plugin@v4"),
        [
            "--git",
            "https://gitlab.com/group/sub_group/Plugin",
            "--tag=v4"
        ],
    );
    assert_eq!(
        arguments("bitbucket.org:workspace/Plugin@abcdef0"),
        [
            "--git",
            "https://bitbucket.org/workspace/Plugin",
            "--rev=abcdef0"
        ],
    );
    assert_eq!(
        arguments("crates.io:9_Mixed-Name@1.2.3"),
        ["--registry", "crates-io", "--", "9_Mixed-Name@1.2.3"],
    );
    assert_eq!(
        arguments("crates.io:--root=/elsewhere"),
        ["--registry", "crates-io", "--", "--root=/elsewhere"],
    );
    let path = std::path::absolute("plug-in fixture").unwrap();
    assert_eq!(
        arguments("local:plug-in fixture"),
        [OsString::from("--path"), path.into_os_string()]
    );
}

#[test]
fn resolves_an_explicit_sparse_index_and_a_configured_alias() {
    let settings: crate::config::Settings = toml::from_str(
        r#"[registeries]
"example.org" = "sparse+https://crates.example.org"
"#,
    )
    .unwrap();
    let registries = settings.registries;
    for (source, name, index) in [
        (
            "sparse-index:crates.example.org/index",
            Some("name:Plugin_Name@1.2.3"),
            "sparse+https://crates.example.org/index/",
        ),
        (
            "example.org:Plugin_Name@1.2.3",
            None,
            "sparse+https://crates.example.org/",
        ),
    ] {
        let source =
            InstallSource::parse(OsStr::new(source), name.map(OsStr::new), &registries).unwrap();
        let mut cargo = Command::new("cargo");
        source.configure_cargo(&mut cargo).unwrap();
        assert_eq!(
            cargo.get_args().collect::<Vec<_>>(),
            ["--index", index, "--", "Plugin_Name@1.2.3"]
        );
    }
    for name in [None, Some("Plugin"), Some("name:")] {
        assert!(InstallSource::parse(
            OsStr::new("sparse-index:crates.example.org/index"),
            name.map(OsStr::new),
            &registries,
        )
        .is_err());
    }
}
