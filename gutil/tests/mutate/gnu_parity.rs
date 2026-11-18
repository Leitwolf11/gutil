use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, PartialEq, Eq)]
enum Entry {
    Directory,
    File(Vec<u8>),
    Symbolic(PathBuf),
}

fn snapshot(root: &Path, directory: &Path) -> BTreeMap<PathBuf, Entry> {
    let mut result = BTreeMap::new();
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        let metadata = std::fs::symlink_metadata(&path).unwrap();
        let record = if metadata.is_symlink() {
            Entry::Symbolic(std::fs::read_link(&path).unwrap())
        } else if metadata.is_dir() {
            result.extend(snapshot(root, &path));
            Entry::Directory
        } else {
            Entry::File(std::fs::read(&path).unwrap())
        };
        result.insert(path.strip_prefix(root).unwrap().to_path_buf(), record);
    }
    result
}

fn hard_links(root: &Path, records: &BTreeMap<PathBuf, Entry>) -> Vec<(PathBuf, PathBuf)> {
    let files: Vec<_> = records
        .iter()
        .filter_map(|(name, entry)| matches!(entry, Entry::File(_)).then_some(name))
        .collect();
    let mut pairs = Vec::new();
    for (index, left) in files.iter().enumerate() {
        for right in &files[index + 1..] {
            if same_file::is_same_file(root.join(left), root.join(right)).unwrap() {
                pairs.push(((*left).clone(), (*right).clone()));
            }
        }
    }
    pairs
}

fn prepare(root: &Path) {
    for (name, bytes) in [
        ("a", b"abcdef".as_slice()),
        ("b", b"old"),
        ("reference", b"reference"),
    ] {
        std::fs::write(root.join(name), bytes).unwrap();
    }
    std::fs::create_dir_all(root.join("dir/sub")).unwrap();
    std::fs::create_dir(root.join("empty")).unwrap();
    for (source, target) in [
        ("a", "symlink"),
        ("missing", "dangling"),
        ("dir", "dirlink"),
    ] {
        std::os::unix::fs::symlink(source, root.join(target)).unwrap();
    }
}

#[test]
#[ignore = "requires GNU Coreutils 9.12 binaries in GNU_COREUTILS_DIR"]
fn filesystem_state_and_exit_codes_match_gnu() {
    let directory = std::env::var_os("GNU_COREUTILS_DIR").unwrap();
    let reference = Path::new(&directory);
    for utility in ["link", "ln", "unlink", "rmdir", "truncate"] {
        let version = Command::new(reference.join(utility))
            .arg("--version")
            .output()
            .unwrap();
        assert!(version.status.success());
        assert!(String::from_utf8_lossy(&version.stdout)
            .lines()
            .next()
            .unwrap()
            .ends_with(" 9.12"));
    }
    let cases = cases();
    for (utility, args) in &cases {
        let fixture = tempfile::tempdir().unwrap();
        let actual = fixture.path().join("actual");
        let expected = fixture.path().join("expected");
        std::fs::create_dir(&actual).unwrap();
        std::fs::create_dir(&expected).unwrap();
        prepare(&actual);
        prepare(&expected);
        let got = super::run(&actual, utility, args);
        let wanted = Command::new(reference.join(utility))
            .current_dir(&expected)
            .args(args)
            .env("LC_ALL", "C")
            .env_remove("VERSION_CONTROL")
            .env_remove("SIMPLE_BACKUP_SUFFIX")
            .output()
            .unwrap();
        assert_eq!(
            got.status.code(),
            wanted.status.code(),
            "{utility} {args:?}"
        );
        assert_eq!(got.stdout, wanted.stdout, "{utility} {args:?}");
        let got = snapshot(&actual, &actual);
        let wanted = snapshot(&expected, &expected);
        assert_eq!(got, wanted, "{utility} {args:?}");
        assert_eq!(
            hard_links(&actual, &got),
            hard_links(&expected, &wanted),
            "{utility} {args:?}"
        );
    }
    assert_eq!(cases.len(), 101);
}

fn cases() -> Vec<(&'static str, Vec<&'static str>)> {
    let mut cases = Vec::new();
    for size in [
        "0",
        "1",
        "+2",
        "-2",
        "<2",
        ">9",
        "/4",
        "%4",
        "-999",
        "+0",
        "1K",
        "1kB",
        "1m",
        "1MiB",
        "K",
        "0Q",
        "+0Q",
        "+K",
        "-K",
        "< K",
        "% K",
        "/0",
        "%0",
        "1b",
        "",
        "-9223372036854775808",
        " 2",
        "+ 2",
        "0b",
        "  +2",
        "9223372036854775808",
    ] {
        cases.push(("truncate", vec!["-s", size, "a"]));
    }
    for args in [
        &["-s+1", "a", "a"][..],
        &["-s2", "dir", "after"],
        &["-c", "-s2", "missing"],
        &["-cs2", "dangling"],
        &["-rreference", "a", "new"],
        &["-rreference", "-s+2", "a"],
        &["-rreference", "-s2", "new"],
        &["-os2", "a"],
        &["-os+2", "-rreference", "a"],
        &["-rmissing", "a"],
    ] {
        cases.push(("truncate", args.to_vec()));
    }
    for args in [
        &["-p", "dir/sub"][..],
        &["--ignore-fail-on-non-empty", "dir"],
        &["--ignore-fail-on-non-empty", "missing"],
        &["-p", "dir/./sub"],
        &["-p", "dir//sub/"],
        &["empty", "missing"],
        &["dirlink"],
        &["a"],
    ] {
        cases.push(("rmdir", args.to_vec()));
    }
    for args in [
        &["a", "new"][..],
        &["missing", "new"],
        &["symlink", "new"],
        &["dir", "new"],
        &["a", "b"],
        &["a", "b", "c"],
        &["--", "-missing", "new"],
    ] {
        cases.push(("link", args.to_vec()));
    }
    for args in [
        &["a"][..],
        &["missing"],
        &["dir"],
        &["dirlink"],
        &["dangling"],
        &["a", "b"],
    ] {
        cases.push(("unlink", args.to_vec()));
    }
    cases.extend(link_cases());
    cases
}

fn link_cases() -> Vec<(&'static str, Vec<&'static str>)> {
    let mut cases = Vec::new();
    for args in [
        &["a", "new"][..],
        &["a"],
        &["a", "b", "empty"],
        &["-s", "missing", "new"],
        &["-sf", "a", "a"],
        &["-sf", "a", "b"],
        &["-sf", "missing/file", "a"],
        &["-sf", "missing/a", "a"],
        &["-sb", "missing/a", "a"],
        &["-f", "a", "b"],
        &["-b", "a", "b"],
        &["-b", "a", "a"],
        &["-S.bak", "a", "b"],
        &["--backup=numbered", "a", "b"],
        &["--backup=existing", "a", "b"],
        &["--backup=off", "a", "b"],
        &["-sbT", "a", "dir"],
        &["-sfn", "a", "dirlink"],
        &["-s", "a", "dirlink"],
        &["-L", "symlink", "new"],
        &["-P", "dangling", "new"],
        &["-sr", "a", "empty/new"],
        &["-srf", "a", "empty/new"],
        &["-t", "empty", "a", "b"],
        &["-r", "a", "new"],
        &["-T", "a"],
        &["-t", "a", "b"],
        &["-stempty", "a", "dangling"],
        &["-sT", "a", "dirlink"],
        &["-sf", "symlink", "a"],
        &["-Pf", "symlink", "a"],
        &["-sf", "symlink", "symlink"],
        &["-sf", "a", "symlink"],
        &["-srv", "a", "empty/new"],
        &["-Lv", "symlink", "new"],
        &["-ft", "empty", "a", "dir/../a"],
        &["--backup=numbered", "-ft", "empty", "a", "dir/../a"],
        &["-bft", "empty", "a", "dir/../a"],
        &["-srft", "empty", "a", "dir/../a"],
    ] {
        cases.push(("ln", args.to_vec()));
    }
    cases
}
