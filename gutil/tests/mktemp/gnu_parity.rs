use super::*;

#[test]
fn compares_creation_state_location_and_templates_with_gnu() {
    let Some(directory) = std::env::var_os("GNU_COREUTILS_DIR") else {
        return;
    };
    let gnu = Path::new(&directory).join("mktemp");
    let fixture = tempfile::tempdir().unwrap();
    let mut count = 0;
    for args in CASES {
        let base = fixture.path().join(count.to_string());
        fs::create_dir(&base).unwrap();
        let mut observations = Vec::new();
        for reference in [false, true] {
            let root = base.join(if reference { "gnu" } else { "gutil" });
            fs::create_dir(&root).unwrap();
            for directory in ["environment", "explicit", "parent"] {
                fs::create_dir(root.join(directory)).unwrap();
            }
            fs::write(root.join("blocked"), b"preserved").unwrap();
            let mut command = Command::new("sh");
            command
                .current_dir(&root)
                .env("LC_ALL", "C")
                .env("TMPDIR", root.join("environment"))
                .args(["-c", "umask 022; exec \"$@\"", "mktemp-comparison"]);
            if reference {
                command.arg(&gnu);
            } else {
                command.arg(env!("CARGO_BIN_EXE_gutil")).args([
                    "--no-config",
                    "--color=never",
                    "--progress=never",
                    "--jobs=1",
                    "mktemp",
                ]);
            }
            let output = command.args(*args).output().unwrap();
            observations.push(observe(&root, &output, args));
            assert_eq!(fs::read(root.join("blocked")).unwrap(), b"preserved");
        }
        assert_eq!(observations[0], observations[1], "{args:?}");
        count += 1;
    }
    assert_eq!(count, CASES.len());
}

#[derive(Debug, PartialEq, Eq)]
struct Observation {
    success: bool,
    parent: Option<PathBuf>,
    length: usize,
    kind: Option<bool>,
    permissions: Option<u32>,
}

fn observe(root: &Path, output: &Output, args: &[&str]) -> Observation {
    use std::os::unix::fs::PermissionsExt;
    if !output.status.success() {
        assert_eq!(output.stdout, b"", "{args:?}: {output:?}");
        return Observation {
            success: false,
            parent: None,
            length: 0,
            kind: None,
            permissions: None,
        };
    }
    let printed = path(output);
    let full = root.join(&printed);
    let parent = full
        .parent()
        .unwrap()
        .strip_prefix(root)
        .unwrap()
        .to_path_buf();
    let metadata = fs::symlink_metadata(&full);
    let (kind, permissions) = match metadata {
        Ok(metadata) => {
            assert!(metadata.is_file() || metadata.is_dir());
            if metadata.is_file() {
                assert_eq!(metadata.len(), 0);
            }
            (
                Some(metadata.is_dir()),
                Some(metadata.permissions().mode() & 0o7777),
            )
        }
        Err(error) => {
            assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
            (None, None)
        }
    };
    let name = printed.file_name().unwrap().to_str().unwrap();
    let expected = args
        .iter()
        .rev()
        .find(|arg| !arg.starts_with('-') && arg.contains("XXX"))
        .copied()
        .unwrap_or("tmp.XXXXXXXXXX");
    let component = expected.rsplit('/').next().unwrap();
    let end = component.rfind('X').unwrap() + 1;
    let start = component[..end].trim_end_matches('X').len();
    let inferred_suffix = &component[end..];
    let suffix = args
        .iter()
        .find_map(|arg| arg.strip_prefix("--suffix="))
        .unwrap_or(inferred_suffix);
    assert_name(&printed, &component[..start], suffix, end - start);
    Observation {
        success: true,
        parent: Some(parent),
        length: name.len(),
        kind,
        permissions,
    }
}

const CASES: &[&[&str]] = &[
    &[],
    &["local.XXX"],
    &["local.XXXXXX"],
    &["XXX.old.XXXX.txt"],
    &["--suffix=.txt", "local.XXX"],
    &["--suffix=X.json", "local.XXXX"],
    &["--suffix=", "empty.XXX"],
    &["--suffix=.txt"],
    &["-d", "directory.XXX"],
    &["-d", "parent/directory.XXX"],
    &["-u", "name.XXX"],
    &["-du", "directory.XXX"],
    &["-u", "missing/name.XXX"],
    &["--tmpdir", "selected.XXX"],
    &["--tmpdir=explicit", "selected.XXX"],
    &["--tmpdir=", "selected.XXX"],
    &["-p", "explicit", "selected.XXX"],
    &["-p", "", "selected.XXX"],
    &["-p", "explicit"],
    &["-t", "-p", "explicit", "selected.XXX"],
    &["-p", "explicit", "-t", "selected.XXX"],
    &["-t", "selected.XXX"],
    &["--tmpdir=parent", "../traversal.XXX"],
    &["parent/name.XXX"],
    &["missing/name.XXX"],
    &["-q", "missing/name.XXX"],
    &["-d", "missing/name.XXX"],
    &["-p", "blocked", "selected.XXX"],
    &["-u", "blocked/name.XXX"],
    &["name.XX"],
    &["name"],
    &["XXX/leaf"],
    &["XXX/"],
    &["oldXXX.newXX"],
    &["XXX", "extra"],
    &["-p"],
    &["--suffix"],
    &["--suffix=/bad", "XXX"],
    &["--suffix=.txt", "XXX.old"],
    &["--tmpdir", "/absoluteXXX"],
    &["-t", "parent/nameXXX"],
    &["--unknown", "XXX"],
];
