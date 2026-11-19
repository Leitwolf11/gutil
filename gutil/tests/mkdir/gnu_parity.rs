use super::*;

#[cfg(unix)]
#[test]
fn compares_permission_modes_with_gnu_across_masks_and_setgid_inheritance() {
    use std::os::unix::fs::PermissionsExt;
    let Some(directory) = std::env::var_os("GNU_COREUTILS_DIR") else {
        return;
    };
    let gnu = Path::new(&directory).join("mkdir");
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let mut index = 0;
    for inherited in [false, true] {
        for mask in ["000", "022", "027", "0777"] {
            for specification in GNU_MODES {
                index += 1;
                let base = root.join(index.to_string());
                fs::create_dir(&base).unwrap();
                fs::set_permissions(
                    &base,
                    fs::Permissions::from_mode(if inherited { 0o2770 } else { 0o770 }),
                )
                .unwrap();
                let actual = masked(&base, mask, &["-m", specification, "actual"]);
                let expected = Command::new("sh")
                    .current_dir(&base)
                    .env("LC_ALL", "C")
                    .args([
                        "-c",
                        "umask \"$1\"; shift; exec \"$@\"",
                        "mkdir-fixture",
                        mask,
                    ])
                    .arg(&gnu)
                    .args(["-m", specification, "expected"])
                    .output()
                    .unwrap();
                assert_eq!(
                    actual.status.success(),
                    expected.status.success(),
                    "{specification:?}, mask {mask}, inherited {inherited}"
                );
                if expected.status.success() {
                    assert_eq!(
                        mode(&base.join("actual")),
                        mode(&base.join("expected")),
                        "{specification:?}, mask {mask}, inherited {inherited}"
                    );
                } else {
                    assert!(!base.join("actual").exists());
                }
            }
        }
    }
    assert_eq!(index, 408);
}

#[cfg(unix)]
const GNU_MODES: [&str; 51] = [
    "755",
    "00755",
    "0",
    "00000",
    "07777",
    "+w",
    "-w",
    "=rw",
    "=0,u+r",
    "u+r-w+x",
    "g=u",
    "a=rwX",
    "u+t",
    "o+t",
    "u+s,g-s",
    "a=s",
    "g=",
    "u+s",
    "g+s",
    "a+s",
    "g-s",
    "a+t,u+r",
    "u+s-s",
    "o+t-t",
    "+0",
    "-0",
    "+1000",
    "-1000",
    "+2000",
    "-2000",
    "+4000",
    "+1",
    "-1",
    "u+X",
    "a=s-s",
    "a+r,a+t",
    "a=rwx,o+t",
    "a+rwXt",
    "u=rw,g=u,o=g",
    "u+rwx,g-w,o=",
    "u+",
    "+",
    "u+gg",
    "u+rxg",
    "777,u+r",
    "+1-r",
    "",
    ",",
    "u=rw,",
    "888",
    "10000",
];

#[cfg(target_os = "linux")]
#[test]
fn inherited_default_acls_and_parent_owner_access_match_gnu() {
    let Some(directory) = std::env::var_os("GNU_COREUTILS_DIR") else {
        return;
    };
    if Command::new("setfacl").arg("--version").output().is_err() {
        return;
    }
    let gnu = Path::new(&directory).join("mkdir");
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    for (index, args) in [
        vec!["plain"],
        vec!["-m755", "plain"],
        vec!["-p", "parent/leaf"],
        vec!["-p", "-m755", "parent/leaf"],
        vec!["-p", "-m000", "parent/leaf"],
    ]
    .into_iter()
    .enumerate()
    {
        let actual = root.join(format!("actual-{index}"));
        let expected = root.join(format!("expected-{index}"));
        for path in [&actual, &expected] {
            fs::create_dir(path).unwrap();
            let result = Command::new("setfacl")
                .args(["-m", "d:u::r-x,d:g::rwx,d:o::---"])
                .arg(path)
                .output()
                .unwrap();
            success(&result);
        }
        let result = masked(&actual, "027", &args);
        let reference = Command::new("sh")
            .current_dir(&expected)
            .env("LC_ALL", "C")
            .args([
                "-c",
                "umask \"$1\"; shift; exec \"$@\"",
                "mkdir-fixture",
                "027",
            ])
            .arg(&gnu)
            .args(&args)
            .output()
            .unwrap();
        assert_eq!(
            result.status, reference.status,
            "{args:?}: {result:?} / {reference:?}"
        );
        for name in ["plain", "parent", "parent/leaf"] {
            assert_eq!(
                actual.join(name).exists(),
                expected.join(name).exists(),
                "{args:?}: {name}"
            );
            if expected.join(name).exists() {
                assert_eq!(
                    mode(&actual.join(name)),
                    mode(&expected.join(name)),
                    "{args:?}: {name}"
                );
            }
        }
    }
}
