//! Files that hold a full copy of a store (exports) are created owner-only
//! (#1668), so another user on the machine cannot read them in the window
//! between creation and a later `chmod`, or at all.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

/// Create `path` for writing with mode 0600 on Unix (default permissions
/// elsewhere).
///
/// With `replace` false the file must not exist yet. With `replace` true an
/// existing file is reused: its mode is tightened to 0600 *before* it is
/// truncated, so old content never stays readable under a looser mode.
pub fn create_private(path: &Path, replace: bool) -> io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.write(true);
    if replace {
        opts.create(true);
    } else {
        opts.create_new(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let file = opts.open(path)?;
    if replace {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        file.set_len(0)?;
    }
    Ok(file)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    fn mode(p: &Path) -> u32 {
        std::fs::metadata(p).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn a_new_file_is_owner_only_whatever_the_umask() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("export.jsonl");
        let mut f = create_private(&p, false).unwrap();
        // Checked while still open: the mode is right from creation.
        assert_eq!(mode(&p), 0o600);
        f.write_all(b"x").unwrap();
    }

    #[test]
    fn a_new_file_must_not_exist() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("e");
        std::fs::write(&p, "keep").unwrap();
        assert!(create_private(&p, false).is_err());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "keep");
    }

    #[test]
    fn replacing_tightens_a_loose_file_and_truncates_it() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("e");
        std::fs::write(&p, "old content").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        drop(create_private(&p, true).unwrap());
        assert_eq!(mode(&p), 0o600);
        assert_eq!(std::fs::metadata(&p).unwrap().len(), 0);
    }
}
