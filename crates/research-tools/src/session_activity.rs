//! Coordinate live turns with portable backup publication across reSearch processes.
use crate::sandbox::{is_symlink_or_junction, validate_session_id};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, ErrorKind};
use std::path::Path;

pub fn pending_name(session: &str) -> String {
    format!(".session-restore-{session}.pending")
}

fn acquire(root: &Path, session: &str, exclusive: bool) -> io::Result<File> {
    validate_session_id(session).map_err(|e| io::Error::new(ErrorKind::InvalidInput, e))?;
    fs::create_dir_all(root)?;
    let path = root.join(format!(".session-activity-{session}.lock"));
    if fs::symlink_metadata(&path).is_ok_and(|meta| is_symlink_or_junction(&path, &meta)) {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "Invalid session reservation path.",
        ));
    }
    // Keep the file even after unlocking: removing it could give two processes different locks.
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    let result = if exclusive {
        file.try_lock()
    } else {
        file.try_lock_shared()
    };
    result.map_err(|e| match e {
        TryLockError::WouldBlock => io::Error::new(
            ErrorKind::WouldBlock,
            format!("Session {session} is busy. Wait for its turn, backup or restore to finish."),
        ),
        TryLockError::Error(err) => err,
    })?;
    Ok(file)
}

pub fn start_turn(root: &Path, session: &str) -> io::Result<File> {
    let file = acquire(root, session, false)?;
    if root.join(pending_name(session)).try_exists()? {
        return Err(io::Error::new(
            ErrorKind::WouldBlock,
            "Finish the interrupted project restore before starting a turn.",
        ));
    }
    Ok(file)
}

pub fn reserve_backup(root: &Path, session: &str) -> io::Result<File> {
    acquire(root, session, true)
}
